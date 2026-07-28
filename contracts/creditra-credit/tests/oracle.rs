use cosmwasm_std::testing::{mock_dependencies, mock_env, mock_info};
use cosmwasm_std::Addr;
use creditra_credit::contract::{execute, instantiate};
use creditra_credit::msg::{ExecuteMsg, InstantiateMsg};

#[test]
fn test_oracle_management() {
    let mut deps = mock_dependencies();
    let env = mock_env();
    let admin = Addr::unchecked("admin");
    let info = mock_info(admin.as_str(), &[]);
    
    instantiate(deps.as_mut(), env.clone(), info.clone(), InstantiateMsg { owner: admin.to_string() }).unwrap();
    
    let oracle1 = Addr::unchecked("oracle1");
    
    // Test add oracle
    let msg = ExecuteMsg::AddOracle { oracle: oracle1.to_string(), weight: 10 };
    execute(deps.as_mut(), env.clone(), info.clone(), msg).unwrap();
    
    // Test report value (should succeed for registered oracle)
    let oracle_info = mock_info(oracle1.as_str(), &[]);
    let msg = ExecuteMsg::ReportValue { value: 100 };
    execute(deps.as_mut(), env.clone(), oracle_info, msg).unwrap();
    
    // Test remove oracle
    let msg = ExecuteMsg::RemoveOracle { oracle: oracle1.to_string() };
    execute(deps.as_mut(), env.clone(), info, msg).unwrap();
}

#[test]
fn test_unauthorized_oracle_management() {
    let mut deps = mock_dependencies();
    let env = mock_env();
    let admin = Addr::unchecked("admin");
    let attacker = Addr::unchecked("attacker");
    let info = mock_info(admin.as_str(), &[]);
    
    instantiate(deps.as_mut(), env.clone(), info.clone(), InstantiateMsg { owner: admin.to_string() }).unwrap();
    
    // Test add oracle by non-admin
    let attacker_info = mock_info(attacker.as_str(), &[]);
    let msg = ExecuteMsg::AddOracle { oracle: "oracle1".to_string(), weight: 10 };
    let res = execute(deps.as_mut(), env.clone(), attacker_info, msg);
    assert!(res.is_err());
}
