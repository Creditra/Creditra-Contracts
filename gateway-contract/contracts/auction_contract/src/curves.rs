// contracts/gateway-auction/src/curves.rs

/// Represents the decay curve used in the Dutch auction
#[derive(Clone, Debug)]
pub enum DecayCurve {
    /// Linear decay: price = start_price - rate * time
    Linear { rate: u128 },

    /// Stepped decay: price reduces every interval
    /// price = start_price - (steps * step_size)
    Stepped { step_size: u128, interval: u64 },

    /// Exponential decay using fixed-point factor
    /// price = start_price * factor^time (scaled)
    Exponential { factor: u128, scale: u128 },
}

/// Errors for curve calculations
#[derive(Debug)]
pub enum CurveError {
    Overflow,
    InvalidInput,
}

/// Calculates price based on selected decay curve
///
/// # Arguments
/// - `start_price`: initial auction price
/// - `elapsed`: time since auction start
/// - `curve`: decay model
///
/// # Returns
/// Result<u128, CurveError>
pub fn calculate_price(
    start_price: u128,
    elapsed: u64,
    curve: &DecayCurve,
) -> Result<u128, CurveError> {
    match curve {
        DecayCurve::Linear { rate } => {
            let decay = rate
                .checked_mul(elapsed as u128)
                .ok_or(CurveError::Overflow)?;

            Ok(start_price.saturating_sub(decay))
        }

        DecayCurve::Stepped { step_size, interval } => {
            if *interval == 0 {
                return Err(CurveError::InvalidInput);
            }

            let steps = elapsed / interval;

            let decay = step_size
                .checked_mul(steps as u128)
                .ok_or(CurveError::Overflow)?;

            Ok(start_price.saturating_sub(decay))
        }

        DecayCurve::Exponential { factor, scale } => {
            if *scale == 0 {
                return Err(CurveError::InvalidInput);
            }

            // fixed-point exponential decay
            let mut price = start_price;

            for _ in 0..elapsed {
                price = price
                    .checked_mul(*factor)
                    .ok_or(CurveError::Overflow)?
                    / *scale;
            }

            Ok(price)
        }
    }
}