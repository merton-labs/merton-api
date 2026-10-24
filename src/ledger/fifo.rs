//! FIFO lot replay (ADR-LEDG-04). Pure: no database or HTTP types.
//!
//! Events are replayed in ledger order. Acquisitions open lots; disposals
//! consume the oldest lots first. Holdings may never go negative.
//!
//! Cost allocation (ADR-NUM-05, Proposed): consuming a whole lot takes its
//! remaining cost exactly. Consuming part of a lot takes
//! `cost × taken / quantity`, rounded half-to-even to
//! [`ALLOCATION_SCALE`] places, and the lot keeps the exact remainder, so
//! total cost is conserved with no rounding drift.

use rust_decimal::{Decimal, RoundingStrategy};

use super::decimal::{Amount, Quantity, SignedAmount, exact_add, exact_sub};

/// Decimal places used for cost allocated out of a partially consumed lot.
pub const ALLOCATION_SCALE: u32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Opens a lot with the given total cost (purchase price plus fees and
    /// taxes, or an opening cost basis).
    Acquire { quantity: Quantity, cost: Amount },
    /// Consumes lots; `proceeds` is the net amount received (may be
    /// negative when fees exceed the gross value).
    Dispose {
        quantity: Quantity,
        proceeds: SignedAmount,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lot {
    /// Index of the acquiring event in the replayed sequence.
    pub opened_by: usize,
    pub quantity: Decimal,
    pub cost: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Disposal {
    pub event: usize,
    pub quantity: Decimal,
    pub cost_basis: Decimal,
    pub proceeds: Decimal,
    /// `proceeds - cost_basis`.
    pub realized: Decimal,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Replay {
    /// Open lots, oldest first.
    pub lots: Vec<Lot>,
    pub disposals: Vec<Disposal>,
    quantity: Decimal,
    cost: Decimal,
}

impl Replay {
    /// Total open quantity, maintained with exact, checked arithmetic.
    pub fn quantity(&self) -> Decimal {
        self.quantity
    }

    /// Total cost of the open lots, maintained with exact, checked arithmetic.
    pub fn cost(&self) -> Decimal {
        self.cost
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FifoError {
    #[error("event {event} sells {requested} units but only {available} are held")]
    InsufficientHoldings {
        event: usize,
        available: Decimal,
        requested: Decimal,
    },
    /// A total would overflow or need rounding to stay representable.
    #[error("event {event} exceeds the supported numeric range")]
    Overflow { event: usize },
}

/// Replays `events` in order and returns the open lots and disposals.
///
/// All running totals use exact arithmetic, so an acquisition that would
/// push them out of range is rejected at that event rather than later.
pub fn replay(events: impl IntoIterator<Item = Event>) -> Result<Replay, FifoError> {
    let mut state = Replay::default();
    for (index, event) in events.into_iter().enumerate() {
        let overflow = FifoError::Overflow { event: index };
        match event {
            Event::Acquire { quantity, cost } => {
                state.quantity = exact_add(state.quantity, quantity.value()).ok_or(overflow)?;
                state.cost = exact_add(state.cost, cost.value()).ok_or(overflow)?;
                state.lots.push(Lot {
                    opened_by: index,
                    quantity: quantity.value(),
                    cost: cost.value(),
                });
            }
            Event::Dispose { quantity, proceeds } => {
                if quantity.value() > state.quantity {
                    return Err(FifoError::InsufficientHoldings {
                        event: index,
                        available: state.quantity,
                        requested: quantity.value(),
                    });
                }
                let disposal = dispose(&mut state.lots, index, quantity.value(), proceeds.value())?;
                state.quantity = exact_sub(state.quantity, disposal.quantity).ok_or(overflow)?;
                state.cost = exact_sub(state.cost, disposal.cost_basis).ok_or(overflow)?;
                state.disposals.push(disposal);
            }
        }
    }
    Ok(state)
}

/// Consumes `requested` units from the oldest lots. The caller has checked
/// that enough units are held.
fn dispose(
    lots: &mut Vec<Lot>,
    event: usize,
    requested: Decimal,
    proceeds: Decimal,
) -> Result<Disposal, FifoError> {
    let overflow = FifoError::Overflow { event };
    let mut remaining = requested;
    let mut cost_basis = Decimal::ZERO;
    let mut consumed = 0;
    for lot in lots.iter_mut() {
        if remaining.is_zero() {
            break;
        }
        let taken = remaining.min(lot.quantity);
        let allocated = if taken == lot.quantity {
            consumed += 1;
            lot.cost
        } else {
            allocate(lot.cost, taken, lot.quantity).ok_or(overflow)?
        };
        lot.quantity = exact_sub(lot.quantity, taken).ok_or(overflow)?;
        lot.cost = exact_sub(lot.cost, allocated).ok_or(overflow)?;
        remaining = exact_sub(remaining, taken).ok_or(overflow)?;
        cost_basis = exact_add(cost_basis, allocated).ok_or(overflow)?;
    }
    lots.drain(..consumed);

    Ok(Disposal {
        event,
        quantity: requested,
        cost_basis,
        proceeds,
        realized: exact_sub(proceeds, cost_basis).ok_or(overflow)?,
    })
}

/// `cost × taken / quantity`, rounded to [`ALLOCATION_SCALE`] places.
/// Multiplies first for precision, falling back to the ratio form when the
/// product would overflow. `taken < quantity`, so the result never exceeds
/// `cost`.
fn allocate(cost: Decimal, taken: Decimal, quantity: Decimal) -> Option<Decimal> {
    let exact = match cost.checked_mul(taken) {
        Some(product) => product.checked_div(quantity)?,
        None => cost.checked_mul(taken.checked_div(quantity)?)?,
    };
    let rounded =
        exact.round_dp_with_strategy(ALLOCATION_SCALE, RoundingStrategy::MidpointNearestEven);
    Some(rounded.min(cost))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(s: &str) -> Quantity {
        Quantity::parse(s).unwrap()
    }
    fn a(s: &str) -> Amount {
        Amount::parse(s).unwrap()
    }
    fn p(s: &str) -> SignedAmount {
        SignedAmount::parse(s).unwrap()
    }
    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }
    fn buy(qty: &str, cost: &str) -> Event {
        Event::Acquire {
            quantity: q(qty),
            cost: a(cost),
        }
    }
    fn sell(qty: &str, proceeds: &str) -> Event {
        Event::Dispose {
            quantity: q(qty),
            proceeds: p(proceeds),
        }
    }

    #[test]
    fn sells_oldest_lots_first() {
        let result = replay([buy("10", "100"), buy("10", "200"), sell("15", "300")]).unwrap();

        assert_eq!(result.disposals[0].cost_basis, d("200")); // 100 + half of 200
        assert_eq!(result.disposals[0].realized, d("100"));
        assert_eq!(result.lots.len(), 1);
        assert_eq!(result.lots[0].opened_by, 1);
        assert_eq!(result.lots[0].quantity, d("5"));
        assert_eq!(result.lots[0].cost, d("100"));
    }

    #[test]
    fn rejects_selling_more_than_held() {
        let err = replay([buy("5", "50"), sell("3", "40"), sell("3", "40")]).unwrap_err();

        assert_eq!(
            err,
            FifoError::InsufficientHoldings {
                event: 2,
                available: d("2"),
                requested: d("3"),
            }
        );
    }

    #[test]
    fn rejects_selling_before_buying() {
        assert!(matches!(
            replay([sell("1", "10"), buy("1", "10")]),
            Err(FifoError::InsufficientHoldings { event: 0, .. })
        ));
    }

    #[test]
    fn partial_allocation_conserves_total_cost() {
        let result = replay([buy("3", "100"), sell("1", "40"), sell("1", "40")]).unwrap();

        assert_eq!(result.disposals[0].cost_basis, d("33.3333333333"));
        assert_eq!(result.disposals[1].cost_basis, d("33.3333333334"));
        let allocated: Decimal = result.disposals.iter().map(|x| x.cost_basis).sum();
        assert_eq!(allocated + result.cost(), d("100"));
        assert_eq!(result.quantity(), d("1"));
    }

    #[test]
    fn closing_a_position_takes_all_remaining_cost() {
        let result = replay([buy("3", "100"), sell("1", "40"), sell("2", "90")]).unwrap();

        let allocated: Decimal = result.disposals.iter().map(|x| x.cost_basis).sum();
        assert_eq!(allocated, d("100"));
        assert!(result.lots.is_empty());
    }

    #[test]
    fn negative_net_proceeds_are_a_loss() {
        let result = replay([buy("1", "10"), sell("1", "-2")]).unwrap();

        assert_eq!(result.disposals[0].realized, d("-12"));
    }

    #[test]
    fn huge_values_do_not_overflow_silently() {
        let max = "999999999999999.99999999";
        let result = replay([buy(max, max), sell("1", "1")]);

        assert!(result.is_ok(), "{result:?}");
        let state = result.unwrap();
        assert!(state.cost() > Decimal::ZERO && state.cost() < d(max));
    }

    #[test]
    fn totals_that_would_overflow_are_rejected_at_the_acquisition() {
        // Each cost fits a Decimal, but their sum does not.
        let huge = Event::Acquire {
            quantity: q("1"),
            cost: Amount::from_stored("50000000000000000000000000000").unwrap(),
        };

        assert_eq!(replay([huge, huge]), Err(FifoError::Overflow { event: 1 }));
    }

    #[test]
    fn fractional_units() {
        let result = replay([buy("0.5", "50.25"), buy("0.25", "30"), sell("0.6", "80")]).unwrap();

        // 0.5 whole (50.25) + 0.1 of 0.25 (30 * 0.1 / 0.25 = 12)
        assert_eq!(result.disposals[0].cost_basis, d("62.25"));
        assert_eq!(result.quantity(), d("0.15"));
        assert_eq!(result.cost(), d("18"));
    }
}
