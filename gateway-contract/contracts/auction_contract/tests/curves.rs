use proptest::prelude::*;
use gateway_auction::{compute_dutch_price, DutchAuctionDecay};

proptest! {
    #[test]
    fn price_never_increases_linear(
        start_price in 1000i128..1_000_000,
        floor_price in 0i128..1000,
        t in 0u64..100
    ) {
        let p1 = compute_dutch_price(
            start_price,
            floor_price,
            t,
            100,
            &DutchAuctionDecay::Linear,
            None
        );

        let p2 = compute_dutch_price(
            start_price,
            floor_price,
            t + 1,
            100,
            &DutchAuctionDecay::Linear,
            None
        );

        prop_assert!(p2 <= p1);
    }
}

proptest! {
    #[test]
    fn price_never_increases_exponential(
        start_price in 1000i128..1_000_000,
        floor_price in 0i128..1000,
        t in 0u64..100
    ) {
        let p1 = compute_dutch_price(
            start_price,
            floor_price,
            t,
            100,
            &DutchAuctionDecay::Exponential,
            None
        );

        let p2 = compute_dutch_price(
            start_price,
            floor_price,
            t + 1,
            100,
            &DutchAuctionDecay::Exponential,
            None
        );

        prop_assert!(p2 <= p1);
    }
}

#[test]
fn test_linear_hits_floor() {
    let price = compute_dutch_price(
        1000,
        100,
        100,
        100,
        &DutchAuctionDecay::Linear,
        None
    );

    assert_eq!(price, 100);
}

#[test]
fn test_stepped_basic() {
    let price = compute_dutch_price(
        1000,
        0,
        50,
        100,
        &DutchAuctionDecay::Stepped,
        Some(2)
    );

    assert!(price <= 1000);
}

#[test]
fn test_exponential_basic() {
    let p1 = compute_dutch_price(
        1000,
        0,
        1,
        100,
        &DutchAuctionDecay::Exponential,
        None
    );

    let p2 = compute_dutch_price(
        1000,
        0,
        2,
        100,
        &DutchAuctionDecay::Exponential,
        None
    );

    assert!(p2 <= p1);
}
#[test]
fn test_zero_time() {
    let price = compute_dutch_price(
        1000,
        100,
        0,
        100,
        &DutchAuctionDecay::Linear,
        None
    );

    assert_eq!(price, 1000);
}

#[test]
fn test_large_time_clamps_to_floor() {
    let price = compute_dutch_price(
        1000,
        100,
        10_000,
        100,
        &DutchAuctionDecay::Linear,
        None
    );

    assert_eq!(price, 100);
}