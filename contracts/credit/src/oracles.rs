// SPDX-License-Identifier: MIT

//! Oracle redundancy module: handles approved oracle signers, weights, reports,
//! and calculating the weighted median value subject to a quorum threshold.

use crate::auth::require_admin_auth;
use crate::events::{
    publish_oracle_added_event, publish_oracle_quorum_threshold_set_event,
    publish_oracle_removed_event, publish_oracle_reporting_window_set_event,
    publish_oracle_value_reported_event,
};
use crate::types::ContractError;
use soroban_sdk::{contracttype, Address, Env, Vec};

/// Maximum number of oracle price feeds accepted in registry and quorum calls.
pub const MAX_ORACLE_FEEDS: u32 = 20;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OracleDataKey {
    OracleList,
    OracleWeight(Address),
    OracleReport(Address),
    QuorumThreshold,
    ReportingWindow,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleReportData {
    pub value: u128,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReportWeight {
    pub value: u128,
    pub weight: u32,
}

/// Adds or updates an oracle's weight in the registry.
/// Admin only.
pub fn add_oracle(env: Env, oracle: Address, weight: u32) {
    require_admin_auth(&env);

    if weight == 0 {
        env.panic_with_error(ContractError::InvalidAmount);
    }

    let mut oracle_list: Vec<Address> = env
        .storage()
        .instance()
        .get(&OracleDataKey::OracleList)
        .unwrap_or_else(|| Vec::new(&env));

    if !oracle_list.contains(&oracle) {
        if oracle_list.len() >= MAX_ORACLE_FEEDS {
            env.panic_with_error(ContractError::OverLimit);
        }
        oracle_list.push_back(oracle.clone());
        env.storage()
            .instance()
            .set(&OracleDataKey::OracleList, &oracle_list);
    }

    env.storage()
        .instance()
        .set(&OracleDataKey::OracleWeight(oracle.clone()), &weight);

    // Emit event
    publish_oracle_added_event(&env, &oracle, weight);
}

/// Removes an oracle from the registry.
/// Admin only.
pub fn remove_oracle(env: Env, oracle: Address) {
    require_admin_auth(&env);

    let mut oracle_list: Vec<Address> = env
        .storage()
        .instance()
        .get(&OracleDataKey::OracleList)
        .unwrap_or_else(|| Vec::new(&env));

    if let Some(idx) = oracle_list.first_index_of(&oracle) {
        oracle_list.remove(idx);
        env.storage()
            .instance()
            .set(&OracleDataKey::OracleList, &oracle_list);

        env.storage()
            .instance()
            .remove(&OracleDataKey::OracleWeight(oracle.clone()));
        env.storage()
            .persistent()
            .remove(&OracleDataKey::OracleReport(oracle.clone()));
        env.storage()
            .instance()
            .remove(&OracleDataKey::OracleReport(oracle.clone()));

        // Emit event
        publish_oracle_removed_event(&env, &oracle);
    } else {
        env.panic_with_error(ContractError::OracleNotFound);
    }
}

/// Sets the quorum threshold.
/// Admin only.
pub fn set_quorum_threshold(env: Env, threshold: u32) {
    require_admin_auth(&env);
    env.storage()
        .instance()
        .set(&OracleDataKey::QuorumThreshold, &threshold);

    // Emit event
    publish_oracle_quorum_threshold_set_event(&env, threshold);
}

/// Returns the configured registry quorum threshold, if any.
///
/// The threshold is the minimum total weight of *fresh* reports that
/// [`get_median_value`] requires before it will return a value. When it is not
/// set the registry is considered inactive and settlement falls back to the
/// quorum-of-K price feed or the single-oracle circuit breaker. See
/// [`crate::oracle_validation`] for the full precedence rules.
pub fn get_registry_quorum_threshold(env: &Env) -> Option<u32> {
    env.storage()
        .instance()
        .get(&OracleDataKey::QuorumThreshold)
}

/// Whether the weighted-median oracle registry is active.
///
/// Registry mode is active once an admin has configured a quorum threshold via
/// [`set_quorum_threshold`]. It takes precedence over both the quorum-of-K
/// price feed and the single-oracle circuit breaker during settlement — see
/// [`crate::oracle_validation`] for the full precedence rules.
pub fn is_registry_configured(env: &Env) -> bool {
    get_registry_quorum_threshold(env).is_some()
}

/// Sets the reporting window.
/// Admin only.
pub fn set_reporting_window(env: Env, window_seconds: u64) {
    require_admin_auth(&env);
    env.storage()
        .instance()
        .set(&OracleDataKey::ReportingWindow, &window_seconds);

    // Emit event
    publish_oracle_reporting_window_set_event(&env, window_seconds);
}

/// Oracles report their observed value.
/// Requires reporting oracle's auth.
pub fn report_value(env: Env, oracle: Address, value: u128) {
    oracle.require_auth();

    if value == 0 {
        env.panic_with_error(ContractError::OraclePriceInvalid);
    }

    // Verify the oracle is registered
    let oracle_list: Vec<Address> = env
        .storage()
        .instance()
        .get(&OracleDataKey::OracleList)
        .unwrap_or_else(|| Vec::new(&env));

    if !oracle_list.contains(&oracle) {
        env.panic_with_error(ContractError::Unauthorized);
    }

    let report = OracleReportData {
        value,
        timestamp: env.ledger().timestamp(),
    };

    let key = OracleDataKey::OracleReport(oracle.clone());
    env.storage().persistent().set(&key, &report);
    crate::storage::bump_persistent_ttl(&env, &key);

    // Emit event
    publish_oracle_value_reported_event(&env, &oracle, value);
}

/// Computes the weighted median of the latest fresh reports from approved oracles.
/// Returns error if quorum threshold is not met or threshold is unset/zero.
pub fn get_median_value(env: Env) -> Result<u128, ContractError> {
    let oracle_list: Vec<Address> = env
        .storage()
        .instance()
        .get(&OracleDataKey::OracleList)
        .unwrap_or_else(|| Vec::new(&env));

    let quorum: u32 = match env
        .storage()
        .instance()
        .get(&OracleDataKey::QuorumThreshold)
    {
        Some(q) if q > 0 => q,
        _ => return Err(ContractError::OracleQuorumNotMet),
    };

    let window: u64 = env
        .storage()
        .instance()
        .get(&OracleDataKey::ReportingWindow)
        .unwrap_or(0);

    let now = env.ledger().timestamp();
    let mut valid_reports = Vec::new(&env);
    let mut total_weight: u32 = 0;

    for oracle in oracle_list.iter() {
        let key = OracleDataKey::OracleReport(oracle.clone());
        if let Some(report) = env.storage().persistent().get::<_, OracleReportData>(&key) {
            crate::storage::bump_persistent_ttl(&env, &key);
            // Freshness check
            if now.saturating_sub(report.timestamp) <= window {
                let weight: u32 = env
                    .storage()
                    .instance()
                    .get(&OracleDataKey::OracleWeight(oracle.clone()))
                    .unwrap_or(0);

                if weight > 0 {
                    valid_reports.push_back(ReportWeight {
                        value: report.value,
                        weight,
                    });
                    total_weight = total_weight
                        .checked_add(weight)
                        .ok_or(ContractError::Overflow)?;
                }
            }
        }
    }

    if total_weight < quorum {
        return Err(ContractError::OracleQuorumNotMet);
    }

    if valid_reports.is_empty() {
        return Err(ContractError::OracleQuorumNotMet);
    }

    // Sort valid reports by value ascending using a simple insertion sort
    let mut reports_arr = valid_reports;
    let len = reports_arr.len();
    for i in 0..len {
        for j in (i + 1)..len {
            let r_i = reports_arr.get_unchecked(i);
            let r_j = reports_arr.get_unchecked(j);
            if r_i.value > r_j.value {
                reports_arr.set(i, r_j);
                reports_arr.set(j, r_i);
            }
        }
    }

    // Find the weighted median
    let target = total_weight.div_ceil(2);
    let mut cumulative_weight: u32 = 0;
    let mut median_value: u128 = 0;

    for report in reports_arr.iter() {
        cumulative_weight = cumulative_weight
            .checked_add(report.weight)
            .ok_or(ContractError::Overflow)?;
        if cumulative_weight >= target {
            median_value = report.value;
            break;
        }
    }

    Ok(median_value)
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{Credit, CreditClient};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::testutils::Ledger as _;
    use soroban_sdk::{Address, Env};

    fn setup_test(env: &Env) -> (CreditClient<'_>, Address) {
        let admin = Address::generate(env);
        let contract_id = env.register(Credit, ());
        let client = CreditClient::new(env, &contract_id);
        client.init(&admin);
        (client, admin)
    }

    #[test]
    fn test_add_oracle_and_weights() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_test(&env);

        let oracle1 = Address::generate(&env);
        let oracle2 = Address::generate(&env);

        client.add_oracle(&oracle1, &10);
        client.add_oracle(&oracle2, &20);

        // Verify registration via storage
        let list: Vec<Address> = env.as_contract(&client.address, || {
            env.storage()
                .instance()
                .get(&OracleDataKey::OracleList)
                .unwrap()
        });
        assert_eq!(list.len(), 2);
        assert!(list.contains(&oracle1));
        assert!(list.contains(&oracle2));

        let w1: u32 = env.as_contract(&client.address, || {
            env.storage()
                .instance()
                .get(&OracleDataKey::OracleWeight(oracle1.clone()))
                .unwrap()
        });
        let w2: u32 = env.as_contract(&client.address, || {
            env.storage()
                .instance()
                .get(&OracleDataKey::OracleWeight(oracle2.clone()))
                .unwrap()
        });
        assert_eq!(w1, 10);
        assert_eq!(w2, 20);

        // Update oracle1 weight
        client.add_oracle(&oracle1, &15);
        let w1_updated: u32 = env.as_contract(&client.address, || {
            env.storage()
                .instance()
                .get(&OracleDataKey::OracleWeight(oracle1))
                .unwrap()
        });
        assert_eq!(w1_updated, 15);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #5)")]
    fn test_add_oracle_zero_weight_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_test(&env);
        let oracle = Address::generate(&env);
        client.add_oracle(&oracle, &0);
    }

    #[test]
    fn test_remove_oracle() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_test(&env);

        let oracle = Address::generate(&env);
        client.add_oracle(&oracle, &10);
        client.report_value(&oracle, &100);
        client.remove_oracle(&oracle);

        let list: Vec<Address> = env.as_contract(&client.address, || {
            env.storage()
                .instance()
                .get(&OracleDataKey::OracleList)
                .unwrap()
        });
        assert_eq!(list.len(), 0);

        let exists_weight = env.as_contract(&client.address, || {
            env.storage()
                .instance()
                .has(&OracleDataKey::OracleWeight(oracle.clone()))
        });
        assert!(!exists_weight);

        let exists_report = env.as_contract(&client.address, || {
            env.storage()
                .persistent()
                .has(&OracleDataKey::OracleReport(oracle))
        });
        assert!(!exists_report);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #55)")]
    fn test_remove_nonexistent_oracle_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_test(&env);
        let oracle = Address::generate(&env);
        client.remove_oracle(&oracle);
    }

    #[test]
    fn test_report_value() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_test(&env);
        let oracle = Address::generate(&env);

        client.add_oracle(&oracle, &10);
        client.report_value(&oracle, &100);

        let report: OracleReportData = env.as_contract(&client.address, || {
            env.storage()
                .persistent()
                .get(&OracleDataKey::OracleReport(oracle.clone()))
                .unwrap()
        });
        assert_eq!(report.value, 100);
        assert_eq!(report.timestamp, env.ledger().timestamp());

        let in_instance = env.as_contract(&client.address, || {
            env.storage()
                .instance()
                .has(&OracleDataKey::OracleReport(oracle))
        });
        assert!(!in_instance);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #1)")]
    fn test_report_unregistered_oracle_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_test(&env);
        let oracle = Address::generate(&env);
        client.report_value(&oracle, &100);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #6)")]
    fn test_add_oracle_exceeding_cap_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_test(&env);

        for _ in 0..MAX_ORACLE_FEEDS {
            let oracle = Address::generate(&env);
            client.add_oracle(&oracle, &10);
        }

        // 21st oracle exceeds MAX_ORACLE_FEEDS (20)
        let oracle_21 = Address::generate(&env);
        client.add_oracle(&oracle_21, &10);
    }

    #[test]
    fn test_add_oracle_update_existing_at_cap_succeeds() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_test(&env);

        let mut first_oracle = None;
        for i in 0..MAX_ORACLE_FEEDS {
            let oracle = Address::generate(&env);
            if i == 0 {
                first_oracle = Some(oracle.clone());
            }
            client.add_oracle(&oracle, &10);
        }

        // Updating weight of already registered oracle at cap succeeds
        let oracle = first_oracle.unwrap();
        client.add_oracle(&oracle, &25);
        let w: u32 = env.as_contract(&client.address, || {
            env.storage()
                .instance()
                .get(&OracleDataKey::OracleWeight(oracle))
                .unwrap()
        });
        assert_eq!(w, 25);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #36)")]
    fn test_report_value_zero_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_test(&env);
        let oracle = Address::generate(&env);

        client.add_oracle(&oracle, &10);
        client.report_value(&oracle, &0);
    }

    #[test]
    fn test_instance_storage_does_not_grow_with_reports() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_test(&env);

        let oracle1 = Address::generate(&env);
        let oracle2 = Address::generate(&env);

        client.add_oracle(&oracle1, &10);
        client.add_oracle(&oracle2, &20);

        client.report_value(&oracle1, &100);
        client.report_value(&oracle2, &200);

        // Verify neither report is in instance storage
        env.as_contract(&client.address, || {
            assert!(!env
                .storage()
                .instance()
                .has(&OracleDataKey::OracleReport(oracle1.clone())));
            assert!(!env
                .storage()
                .instance()
                .has(&OracleDataKey::OracleReport(oracle2.clone())));
            // But they do exist in persistent storage
            assert!(env
                .storage()
                .persistent()
                .has(&OracleDataKey::OracleReport(oracle1)));
            assert!(env
                .storage()
                .persistent()
                .has(&OracleDataKey::OracleReport(oracle2)));
        });
    }

    #[test]
    fn test_get_median_value_threshold_unset_errors() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_test(&env);
        let oracle = Address::generate(&env);

        client.add_oracle(&oracle, &10);
        client.set_reporting_window(&3600);
        client.report_value(&oracle, &100);

        // Quorum threshold is never set
        let res = client.try_get_median_value();
        assert_eq!(res, Err(Ok(ContractError::OracleQuorumNotMet)));
    }

    #[test]
    fn test_get_median_value_threshold_zero_errors() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_test(&env);
        let oracle = Address::generate(&env);

        client.add_oracle(&oracle, &10);
        client.set_quorum_threshold(&0);
        client.set_reporting_window(&3600);
        client.report_value(&oracle, &100);

        let res = client.try_get_median_value();
        assert_eq!(res, Err(Ok(ContractError::OracleQuorumNotMet)));
    }

    #[test]
    fn test_get_median_value_quorum_not_met() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_test(&env);
        let oracle = Address::generate(&env);

        client.add_oracle(&oracle, &10);
        client.set_quorum_threshold(&15);
        client.set_reporting_window(&3600);
        client.report_value(&oracle, &100);

        // Total weight is 10, quorum is 15.
        let res = client.try_get_median_value();
        assert!(res.is_err());
    }

    #[test]
    fn test_get_median_value_stale_reports() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_test(&env);
        let oracle = Address::generate(&env);

        client.add_oracle(&oracle, &10);
        client.set_quorum_threshold(&10);
        client.set_reporting_window(&60); // 60 seconds

        env.ledger().with_mut(|li| li.timestamp = 100);
        client.report_value(&oracle, &100);

        // Advance timestamp by 61 seconds (past window of 60)
        env.ledger().with_mut(|li| li.timestamp = 161);
        let res = client.try_get_median_value();
        assert!(res.is_err());
    }

    #[test]
    fn test_weighted_median_calculations() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_test(&env);

        let oracle1 = Address::generate(&env);
        let oracle2 = Address::generate(&env);
        let oracle3 = Address::generate(&env);

        client.add_oracle(&oracle1, &10); // weight 10
        client.add_oracle(&oracle2, &20); // weight 20
        client.add_oracle(&oracle3, &15); // weight 15

        client.set_quorum_threshold(&45); // total weight is 45
        client.set_reporting_window(&100);

        // Case 1: Oracle reports are 100, 200, 300
        client.report_value(&oracle1, &100);
        client.report_value(&oracle2, &200);
        client.report_value(&oracle3, &300);

        // Sorted: (100, 10), (200, 20), (300, 15)
        // Total weight = 45. Target = (45+1)/2 = 23.
        // Cum weight: 100 (10), 200 (10+20=30 >= 23).
        // Median should be 200.
        let val = client.get_median_value();
        assert_eq!(val, 200);

        // Case 2: Oracle reports are 300, 100, 200
        client.report_value(&oracle1, &300); // (300, 10)
        client.report_value(&oracle2, &100); // (100, 20)
        client.report_value(&oracle3, &200); // (200, 15)
                                             // Sorted: (100, 20), (200, 15), (300, 10)
                                             // Total weight = 45, Target = 23.
                                             // Cum weight: 100 (20), 200 (20+15=35 >= 23).
                                             // Median should be 200.
        let val = client.get_median_value();
        assert_eq!(val, 200);

        // Case 3: High weight dominates
        client.add_oracle(&oracle2, &40); // weight 40
        client.set_quorum_threshold(&65); // total weight: 10 + 40 + 15 = 65
        client.report_value(&oracle1, &500); // weight 10
        client.report_value(&oracle2, &150); // weight 40
        client.report_value(&oracle3, &900); // weight 15
                                             // Sorted: (150, 40), (500, 10), (900, 15)
                                             // Total weight = 65. Target = (65+1)/2 = 33.
                                             // Cum weight: 150 (40 >= 33).
                                             // Median should be 150.
        let val = client.get_median_value();
        assert_eq!(val, 150);
    }

    // --- Proptests and Reference Implementation ---
    use proptest::collection::vec as prop_vec;
    use proptest::prelude::*;

    /// Pure Rust reference implementation for weighted median
    fn reference_median(reports: &[(u128, u32)]) -> Option<u128> {
        if reports.is_empty() {
            return None;
        }
        let mut total_weight = 0u64; // use u64 to avoid overflow during sum
        let mut sorted = reports.to_vec();
        sorted.sort_by_key(|r| r.0);

        for (_, w) in &sorted {
            total_weight += *w as u64;
        }
        if total_weight == 0 {
            return None;
        }

        let target = (total_weight + 1) / 2; // div_ceil(2)
        let mut cumulative = 0u64;
        for (v, w) in sorted {
            cumulative += w as u64;
            if cumulative >= target {
                return Some(v);
            }
        }
        None
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_random_inputs(
            weights in prop_vec(1..=1_000_000u32, 1..=20),
            values in prop_vec(1..=1_000_000_000u128, 1..=20)
        ) {
            let env = Env::default();
            env.mock_all_auths();
            let (client, _) = setup_test(&env);

            let n = std::cmp::min(weights.len(), values.len());
            let mut valid_reports = std::vec::Vec::new();
            let mut total_weight = 0u32;

            for i in 0..n {
                let addr = Address::generate(&env);
                let w = weights[i];
                let v = values[i];
                client.add_oracle(&addr, &w);
                client.report_value(&addr, &v);

                valid_reports.push((v, w));
                total_weight += w;
            }

            client.set_quorum_threshold(&(total_weight / 2));
            client.set_reporting_window(&1000);

            let contract_median = client.get_median_value();
            let ref_median = reference_median(&valid_reports).unwrap();

            assert_eq!(contract_median, ref_median);
        }

        #[test]
        fn prop_equal_weights(
            values in prop_vec(1..=1_000_000_000u128, 1..=20)
        ) {
            let env = Env::default();
            env.mock_all_auths();
            let (client, _) = setup_test(&env);

            let n = values.len();
            let mut valid_reports = std::vec::Vec::new();

            for i in 0..n {
                let addr = Address::generate(&env);
                let w = 100u32;
                let v = values[i];
                client.add_oracle(&addr, &w);
                client.report_value(&addr, &v);

                valid_reports.push((v, w));
            }

            client.set_quorum_threshold(&(n as u32 * 100 / 2));
            client.set_reporting_window(&1000);

            let contract_median = client.get_median_value();
            let ref_median = reference_median(&valid_reports).unwrap();

            assert_eq!(contract_median, ref_median);
        }

        #[test]
        fn prop_single_dominant_weight(
            mut weights in prop_vec(1..=10_000u32, 1..=10),
            values in prop_vec(1..=1_000_000_000u128, 1..=10)
        ) {
            let env = Env::default();
            env.mock_all_auths();
            let (client, _) = setup_test(&env);

            let n = std::cmp::min(weights.len(), values.len());

            // Make the first weight dominant
            let sum: u32 = weights.iter().skip(1).sum();
            weights[0] = sum + 1;

            let mut valid_reports = std::vec::Vec::new();
            let mut total_weight = 0u32;

            for i in 0..n {
                let addr = Address::generate(&env);
                let w = weights[i];
                let v = values[i];
                client.add_oracle(&addr, &w);
                client.report_value(&addr, &v);

                valid_reports.push((v, w));
                total_weight += w;
            }

            client.set_quorum_threshold(&(total_weight / 2));
            client.set_reporting_window(&1000);

            let contract_median = client.get_median_value();
            let ref_median = reference_median(&valid_reports).unwrap();

            assert_eq!(contract_median, ref_median);
            assert_eq!(contract_median, values[0]); // dominant weight dictates median
        }

        #[test]
        fn prop_stale_reports_excluded(
            weights in prop_vec(1..=10_000u32, 2..=10),
            values in prop_vec(1..=1_000_000_000u128, 2..=10)
        ) {
            let env = Env::default();
            env.mock_all_auths();
            let (client, _) = setup_test(&env);

            let n = std::cmp::min(weights.len(), values.len());
            let mut valid_reports = std::vec::Vec::new();
            let mut total_weight = 0u32;

            for i in 0..n {
                let addr = Address::generate(&env);
                let w = weights[i];
                let v = values[i];
                client.add_oracle(&addr, &w);

                if i % 2 == 0 {
                    env.ledger().with_mut(|li| li.timestamp = 0);
                } else {
                    env.ledger().with_mut(|li| li.timestamp = 2000);
                    valid_reports.push((v, w));
                    total_weight += w;
                }

                client.report_value(&addr, &v);
            }

            env.ledger().with_mut(|li| li.timestamp = 2500);

            client.set_quorum_threshold(&(total_weight / 2));
            client.set_reporting_window(&1000);

            if valid_reports.is_empty() {
                assert!(client.try_get_median_value().is_err());
            } else {
                let contract_median = client.get_median_value();
                let ref_median = reference_median(&valid_reports).unwrap();
                assert_eq!(contract_median, ref_median);
            }
        }
    }

    #[test]
    fn test_removed_oracle_ignored() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _) = setup_test(&env);

        let o1 = Address::generate(&env);
        let o2 = Address::generate(&env);

        client.add_oracle(&o1, &100);
        client.add_oracle(&o2, &200);

        client.report_value(&o1, &10);
        client.report_value(&o2, &50);

        client.set_quorum_threshold(&100);
        client.set_reporting_window(&1000);

        assert_eq!(client.get_median_value(), 50);

        client.remove_oracle(&o2);

        assert_eq!(client.get_median_value(), 10);
    }
}

// # Multi-oracle quorum price resolution
//
// Implements the quorum-of-K algorithm for combining multiple independent
// oracle price feeds into a single canonical price used by
// [`crate::lib::settle_default_liquidation`].
//
// ## Algorithm
//
// Given N submitted prices and a quorum threshold K:
//
// 1. Validate every price is strictly positive and N ≤ [`MAX_ORACLE_FEEDS`].
// 2. Sort prices ascending (selection sort; O(n²) but bounded by
//    [`MAX_ORACLE_FEEDS`] ≤ 20 to keep gas predictable).
// 3. Slide a window of K consecutive prices over the sorted array.
// 4. For each window, check whether the highest price deviates from the
//    lowest by no more than `max_deviation_bps` of the lowest.
// 5. Return the **lower-median** of the first qualifying window.
// 6. Panic with [`crate::types::ContractError::OracleQuorumNotMet`] if no
//    window qualifies.
//
// ## Security properties
//
// - An outlier feed cannot influence the result unless it falls inside a
//   qualifying K-wide window alongside K−1 honest feeds.
// - Requires at least K feeds to agree, so an attacker must corrupt K
//   independent feeds simultaneously to manipulate the canonical price.
// - The stack buffer is bounded at compile time; gas consumption is O(n²)
//   for sorting and O(n) for window scanning.

use crate::math_utils::compute_deviation_bps;
use crate::types::OracleQuorumConfig;

/// Resolve a single canonical price from N submitted oracle prices using
/// the quorum-of-K sliding-window algorithm.
///
/// # Parameters
/// - `env`: Soroban host environment (used to panic with typed errors).
/// - `prices`: N submitted prices in any order, one per oracle feed.
/// - `cfg`: Quorum configuration supplying K, max deviation, and max age.
///
/// # Returns
/// The lower-median price of the first K-wide consecutive window (in sorted
/// ascending order) whose highest-to-lowest spread is within
/// `cfg.max_deviation_bps`.
///
/// # Errors
///
/// Panics with [`ContractError::OraclePriceInvalid`] when:
/// - The price list is empty.
/// - The price list exceeds [`MAX_ORACLE_FEEDS`].
/// - Any individual price is ≤ 0.
///
/// Panics with [`ContractError::OracleQuorumNotMet`] when:
/// - `min_quorum_k < 2` (a single feed is not a meaningful quorum).
/// - `min_quorum_k > n` (cannot form a window larger than the input).
/// - No K-wide window in the sorted array satisfies the deviation bound.
pub fn resolve_quorum_price(env: &Env, prices: &Vec<i128>, cfg: &OracleQuorumConfig) -> i128 {
    let n = prices.len();

    if n == 0 || n > MAX_ORACLE_FEEDS {
        env.panic_with_error(ContractError::OraclePriceInvalid);
    }

    let k = cfg.min_quorum_k;
    if k < 2 || k > n {
        env.panic_with_error(ContractError::OracleQuorumNotMet);
    }

    // Copy prices into a fixed stack buffer and validate positivity.
    let mut buf = [0i128; MAX_ORACLE_FEEDS as usize];
    for i in 0..n {
        let p = prices
            .get(i)
            .unwrap_or_else(|| env.panic_with_error(ContractError::OraclePriceInvalid));
        if p <= 0 {
            env.panic_with_error(ContractError::OraclePriceInvalid);
        }
        buf[i as usize] = p;
    }
    let slice = &mut buf[..n as usize];

    // Selection sort — O(n²), safe and predictable for n ≤ MAX_ORACLE_FEEDS.
    let len = slice.len();
    for i in 0..len {
        let mut min_idx = i;
        for j in (i + 1)..len {
            if slice[j] < slice[min_idx] {
                min_idx = j;
            }
        }
        slice.swap(i, min_idx);
    }

    // Scan every consecutive K-wide window in sorted order.
    let kk = k as usize;
    for i in 0..=(len - kk) {
        let lo = slice[i];
        let hi = slice[i + kk - 1];
        let dev = compute_deviation_bps(hi, lo).unwrap_or(u32::MAX);
        if dev <= cfg.max_deviation_bps {
            let median_idx = i + (kk - 1) / 2;
            return slice[median_idx];
        }
    }

    env.panic_with_error(ContractError::OracleQuorumNotMet)
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::OracleQuorumConfig;
    use soroban_sdk::{vec, Env, Vec};

    fn cfg(k: u32, dev: u32) -> OracleQuorumConfig {
        OracleQuorumConfig {
            min_quorum_k: k,
            max_deviation_bps: dev,
            max_age_seconds: 3_600,
        }
    }

    // ── happy-path ────────────────────────────────────────────────────────────

    #[test]
    fn two_of_two_exact_match_returns_lower() {
        let env = Env::default();
        let prices = vec![&env, 1_000i128, 1_000i128];
        assert_eq!(resolve_quorum_price(&env, &prices, &cfg(2, 0)), 1_000);
    }

    #[test]
    fn two_of_three_outlier_ignored() {
        let env = Env::default();
        let prices = vec![&env, 2_000i128, 1_000i128, 1_040i128];
        assert_eq!(resolve_quorum_price(&env, &prices, &cfg(2, 500)), 1_000);
    }

    #[test]
    fn three_of_five_returns_median_of_window() {
        let env = Env::default();
        let prices = vec![&env, 1_000i128, 5_000i128, 980i128, 990i128, 1_010i128];
        assert_eq!(resolve_quorum_price(&env, &prices, &cfg(3, 500)), 990);
    }

    #[test]
    fn all_identical_prices_zero_deviation() {
        let env = Env::default();
        let prices = vec![&env, 500i128, 500i128, 500i128];
        assert_eq!(resolve_quorum_price(&env, &prices, &cfg(3, 0)), 500);
    }

    #[test]
    fn window_at_end_of_sorted_array() {
        let env = Env::default();
        let prices = vec![&env, 2_010i128, 1_000i128, 2_000i128];
        assert_eq!(resolve_quorum_price(&env, &prices, &cfg(2, 100)), 2_000);
    }

    #[test]
    fn four_of_four_returns_lower_median() {
        let env = Env::default();
        let prices = vec![&env, 130i128, 100i128, 120i128, 110i128];
        assert_eq!(resolve_quorum_price(&env, &prices, &cfg(4, 5_000)), 110);
    }

    #[test]
    fn two_of_two_within_boundary_bps() {
        let env = Env::default();
        let prices = vec![&env, 1_050i128, 1_000i128];
        assert_eq!(resolve_quorum_price(&env, &prices, &cfg(2, 500)), 1_000);
    }

    // ── error paths ───────────────────────────────────────────────────────────

    #[test]
    #[should_panic]
    fn empty_prices_panics() {
        let env = Env::default();
        let empty: Vec<i128> = Vec::new(&env);
        resolve_quorum_price(&env, &empty, &cfg(2, 500));
    }

    #[test]
    #[should_panic]
    fn negative_price_panics() {
        let env = Env::default();
        let prices = vec![&env, 1_000i128, -1i128, 1_010i128];
        resolve_quorum_price(&env, &prices, &cfg(2, 500));
    }

    #[test]
    #[should_panic]
    fn zero_price_panics() {
        let env = Env::default();
        let prices = vec![&env, 1_000i128, 0i128];
        resolve_quorum_price(&env, &prices, &cfg(2, 500));
    }

    #[test]
    #[should_panic]
    fn k_greater_than_n_panics() {
        let env = Env::default();
        let prices = vec![&env, 1_000i128, 1_010i128];
        resolve_quorum_price(&env, &prices, &cfg(3, 500));
    }

    #[test]
    #[should_panic]
    fn k_equals_one_panics() {
        let env = Env::default();
        let prices = vec![&env, 1_000i128, 1_010i128];
        resolve_quorum_price(&env, &prices, &cfg(1, 500));
    }

    #[test]
    #[should_panic]
    fn k_equals_zero_panics() {
        let env = Env::default();
        let prices = vec![&env, 1_000i128, 1_010i128];
        resolve_quorum_price(&env, &prices, &cfg(0, 500));
    }

    #[test]
    #[should_panic]
    fn no_qualifying_window_panics() {
        let env = Env::default();
        let prices = vec![&env, 1_000i128, 2_000i128, 4_000i128];
        resolve_quorum_price(&env, &prices, &cfg(2, 500));
    }

    #[test]
    #[should_panic]
    fn just_over_deviation_bound_panics() {
        let env = Env::default();
        let prices = vec![&env, 1_051i128, 1_000i128];
        resolve_quorum_price(&env, &prices, &cfg(2, 500));
    }
}
