use credit::cross_chain::{BridgeAttestation, CrossChainHook};

fn sample_att() -> BridgeAttestation {
    BridgeAttestation {
        user: "alice".to_string(),
        debt_amount: 1000,
        source_chain: 1,
        nonce: 42,
        signature: vec![1, 2, 3],
    }
}

#[test]
fn test_valid_attestation_executes_liquidation() {
    let mut hook = CrossChainHook::new("admin".to_string());

    let res = hook.process_attestation("admin", sample_att());
    assert!(res.is_ok());
    assert!(res.unwrap());
}

#[test]
fn test_unauthorized_rejected() {
    let mut hook = CrossChainHook::new("admin".to_string());

    let res = hook.process_attestation("hacker", sample_att());
    assert!(res.is_err());
}

#[test]
fn test_replay_attack_blocked() {
    let mut hook = CrossChainHook::new("admin".to_string());

    let att = sample_att();

    let _ = hook.process_attestation("admin", att.clone());
    let res = hook.process_attestation("admin", att);

    assert!(matches!(
        res,
        Err(credit::cross_chain::CrossChainError::ReplayAttack)
    ));
}

#[test]
fn test_invalid_signature() {
    let mut hook = CrossChainHook::new("admin".to_string());

    let mut att = sample_att();
    att.signature = vec![];

    let res = hook.process_attestation("admin", att);
    assert!(res.is_err());
}
