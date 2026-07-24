use gateway_auction::{calculate_price, CurveError, DecayCurve};

#[test]
fn test_dutch_stepped_boundary_prices() {
    let start = 1200u128;
    let floor = 600u128;
    let duration = 3600u64;
    let steps = 6u128;

    let curve = DecayCurve::DutchStepped { floor_price: floor, steps, duration };

    assert_eq!(calculate_price(start, 0, &curve).unwrap(), 1200);
    assert_eq!(calculate_price(start, 599, &curve).unwrap(), 1200);
    assert_eq!(calculate_price(start, 600, &curve).unwrap(), 1100);
    assert_eq!(calculate_price(start, 3599, &curve).unwrap(), 700);
    assert_eq!(calculate_price(start, 3600, &curve).unwrap(), 600);
}

#[test]
fn test_dutch_stepped_invalid_inputs() {
    let start = 1000u128;
    // zero steps is invalid
    let curve = DecayCurve::DutchStepped { floor_price: 500, steps: 0, duration: 100 };
    assert!(matches!(calculate_price(start, 10, &curve), Err(CurveError::InvalidInput)));

    // zero duration is invalid
    let curve2 = DecayCurve::DutchStepped { floor_price: 500, steps: 5, duration: 0 };
    assert!(matches!(calculate_price(start, 10, &curve2), Err(CurveError::InvalidInput)));
}
