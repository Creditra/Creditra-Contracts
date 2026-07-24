// contracts/gateway-auction/src/curves.rs

/// Represents the decay curve used in the Dutch auction
#[derive(Clone, Debug)]
pub enum DecayCurve {
    /// Linear decay: price = start_price - rate * time
    Linear { rate: u128 },

    /// Stepped decay: price reduces every interval
    /// price = start_price - (steps * step_size)

    Stepped { step_size: u128, interval: u64 },

    /// Dutch-style stepped decay that splits the total drop from
    /// `start_price` down to `floor_price` across `steps` equal-duration
    /// buckets. The price is constant within a bucket and only drops at
    /// bucket boundaries.
    ///
    /// The variant stores the `floor_price` and `steps` (number of buckets).
    /// `duration` is provided to `calculate_price` via the variant to allow
    /// computing bucket indices safely inside the curve implementation.
    DutchStepped { floor_price: u128, steps: u128, duration: u64 },

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

        DecayCurve::DutchStepped { floor_price, steps, duration } => {
            if *duration == 0 || *steps == 0 {
                return Err(CurveError::InvalidInput);
            }

            // total drop from start to floor
            let total_drop = start_price.saturating_sub(*floor_price);

            // elapsed steps = floor(elapsed * steps / duration)
            let elapsed_steps = (elapsed as u128)
                .checked_mul(*steps)
                .ok_or(CurveError::Overflow)?
                / (*duration as u128);

            let q = total_drop / *steps;
            let r = total_drop % *steps;

            let drop = q
                .checked_mul(elapsed_steps)
                .ok_or(CurveError::Overflow)?
                .checked_add((r.checked_mul(elapsed_steps).ok_or(CurveError::Overflow)? / *steps))
                .ok_or(CurveError::Overflow)?;

            Ok(start_price.saturating_sub(drop))
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