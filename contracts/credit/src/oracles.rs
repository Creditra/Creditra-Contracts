// SPDX-License-Identifier: MIT

//! Oracle redundancy module: handles approved oracle signers, weights, reports,
//! and calculating the weighted median value subject to a quorum threshold.

use crate::auth::require_admin_auth;
use crate::types::ContractError;
use soroban_sdk::{contracttype, Address, Env, Vec};

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
        panic!("Oracle weight must be greater than zero");
    }

    let mut oracle_list: Vec<Address> = env
        .storage()
        .instance()
        .get(&OracleDataKey::OracleList)
        .unwrap_or_else(|| Vec::new(&env));

    if !oracle_list.contains(&oracle) {
        oracle_list.push_back(oracle.clone());
        env.storage().instance().set(&OracleDataKey::OracleList, &oracle_list);
    }

    env.storage().instance().set(&OracleDataKey::OracleWeight(oracle), &weight);
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
        env.storage().instance().set(&OracleDataKey::OracleList, &oracle_list);
        
        env.storage().instance().remove(&OracleDataKey::OracleWeight(oracle.clone()));
        env.storage().instance().remove(&OracleDataKey::OracleReport(oracle));
    } else {
        panic!("Oracle not found in registry");
    }
}

/// Sets the quorum threshold.
/// Admin only.
pub fn set_quorum_threshold(env: Env, threshold: u32) {
    require_admin_auth(&env);
    env.storage().instance().set(&OracleDataKey::QuorumThreshold, &threshold);
}

/// Sets the reporting window.
/// Admin only.
pub fn set_reporting_window(env: Env, window_seconds: u64) {
    require_admin_auth(&env);
    env.storage().instance().set(&OracleDataKey::ReportingWindow, &window_seconds);
}

/// Oracles report their observed value.
/// Requires reporting oracle's auth.
pub fn report_value(env: Env, oracle: Address, value: u128) {
    oracle.require_auth();

    // Verify the oracle is registered
    let oracle_list: Vec<Address> = env
        .storage()
        .instance()
        .get(&OracleDataKey::OracleList)
        .unwrap_or_else(|| Vec::new(&env));

    if !oracle_list.contains(&oracle) {
        panic!("Oracle is not approved");
    }

    let report = OracleReportData {
        value,
        timestamp: env.ledger().timestamp(),
    };

    env.storage().instance().set(&OracleDataKey::OracleReport(oracle), &report);
}

/// Computes the weighted median of the latest fresh reports from approved oracles.
/// Returns error if quorum threshold is not met.
pub fn get_median_value(env: Env) -> Result<u128, ContractError> {
    let oracle_list: Vec<Address> = env
        .storage()
        .instance()
        .get(&OracleDataKey::OracleList)
        .unwrap_or_else(|| Vec::new(&env));

    let quorum: u32 = env
        .storage()
        .instance()
        .get(&OracleDataKey::QuorumThreshold)
        .unwrap_or(0);

    let window: u64 = env
        .storage()
        .instance()
        .get(&OracleDataKey::ReportingWindow)
        .unwrap_or(0);

    let now = env.ledger().timestamp();
    let mut valid_reports = Vec::new(&env);
    let mut total_weight: u32 = 0;

    for oracle in oracle_list.iter() {
        if let Some(report) = env
            .storage()
            .instance()
            .get::<_, OracleReportData>(&OracleDataKey::OracleReport(oracle.clone()))
        {
            // Freshness check
            if now.saturating_sub(report.timestamp) <= window {
                let weight: u32 = env
                    .storage()
                    .instance()
                    .get(&OracleDataKey::OracleWeight(oracle.clone()))
                    .unwrap_or(0);

                if weight > 0 {
                    valid_reports.push_back(ReportWeight { value: report.value, weight });
                    total_weight = total_weight.checked_add(weight).ok_or(ContractError::Overflow)?;
                }
            }
        }
    }

    if total_weight < quorum {
        return Err(ContractError::QuorumNotMet);
    }

    if valid_reports.is_empty() {
        return Err(ContractError::QuorumNotMet);
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
        cumulative_weight = cumulative_weight.checked_add(report.weight).ok_or(ContractError::Overflow)?;
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
    use soroban_sdk::{Env, Address};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::testutils::Ledger as _;

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
            env.storage().instance().get(&OracleDataKey::OracleList).unwrap()
        });
        assert_eq!(list.len(), 2);
        assert!(list.contains(&oracle1));
        assert!(list.contains(&oracle2));

        let w1: u32 = env.as_contract(&client.address, || {
            env.storage().instance().get(&OracleDataKey::OracleWeight(oracle1.clone())).unwrap()
        });
        let w2: u32 = env.as_contract(&client.address, || {
            env.storage().instance().get(&OracleDataKey::OracleWeight(oracle2.clone())).unwrap()
        });
        assert_eq!(w1, 10);
        assert_eq!(w2, 20);
        
        // Update oracle1 weight
        client.add_oracle(&oracle1, &15);
        let w1_updated: u32 = env.as_contract(&client.address, || {
            env.storage().instance().get(&OracleDataKey::OracleWeight(oracle1)).unwrap()
        });
        assert_eq!(w1_updated, 15);
    }

    #[test]
    #[should_panic(expected = "Oracle weight must be greater than zero")]
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
        client.remove_oracle(&oracle);

        let list: Vec<Address> = env.as_contract(&client.address, || {
            env.storage().instance().get(&OracleDataKey::OracleList).unwrap()
        });
        assert_eq!(list.len(), 0);

        let exists = env.as_contract(&client.address, || {
            env.storage().instance().has(&OracleDataKey::OracleWeight(oracle.clone()))
        });
        assert!(!exists);
    }

    #[test]
    #[should_panic(expected = "Oracle not found in registry")]
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
            env.storage().instance().get(&OracleDataKey::OracleReport(oracle)).unwrap()
        });
        assert_eq!(report.value, 100);
        assert_eq!(report.timestamp, env.ledger().timestamp());
    }

    #[test]
    #[should_panic(expected = "Oracle is not approved")]
    fn test_report_unregistered_oracle_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin) = setup_test(&env);
        let oracle = Address::generate(&env);
        client.report_value(&oracle, &100);
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
}
