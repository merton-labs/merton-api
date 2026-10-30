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
//!
//! Splits and stock dividends (ADR-TXN-06/07) change the quantity of every
//! open lot in proportion and keep each lot's cost and date, so total cost
//! is unchanged. A lot's new quantity is rounded half-to-even to
//! [`QUANTITY_SCALE`] places, and the last lot takes the difference so the
//! lots add up to the new total exactly.

use rust_decimal::{Decimal, RoundingStrategy};

use super::decimal::{
    Amount, Quantity, Ratio, SignedAmount, div_round, exact_add, exact_mul, exact_sub,
};

/// Decimal places used for cost allocated out of a partially consumed lot.
pub const ALLOCATION_SCALE: u32 = 10;
/// Decimal places of a holding after a split, the same as entered quantities.
pub const QUANTITY_SCALE: u32 = super::decimal::MAX_SCALE;

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
    /// Multiplies the holding by `new / old`; the new total is rounded to
    /// [`QUANTITY_SCALE`] places.
    Split { ratio: Ratio },
    /// Adds `quantity` shares spread over the open lots, at no cost.
    StockDividend { quantity: Quantity },
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
    /// A split or stock dividend applies to an instrument with no holding.
    #[error("event {event} adjusts a holding that is empty")]
    NoHoldings { event: usize },
    /// A split would round the holding, or one of its lots, to zero.
    #[error("event {event} leaves a lot too small to represent")]
    RoundsToZero { event: usize },
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
            Event::Split { ratio } => {
                let scaled =
                    exact_mul(state.quantity, ratio.new_shares().value()).ok_or(overflow)?;
                let total = div_round(scaled, ratio.old_shares().value(), QUANTITY_SCALE)
                    .ok_or(overflow)?;
                rescale(&mut state, index, total)?;
            }
            Event::StockDividend { quantity } => {
                let total = exact_add(state.quantity, quantity.value()).ok_or(overflow)?;
                rescale(&mut state, index, total)?;
            }
        }
    }
    Ok(state)
}

/// Spreads `total` units over the open lots in proportion to their current
/// quantities. Costs are untouched.
fn rescale(state: &mut Replay, event: usize, total: Decimal) -> Result<(), FifoError> {
    let overflow = FifoError::Overflow { event };
    let held = state.quantity;
    let Some(last) = state.lots.len().checked_sub(1).filter(|_| !held.is_zero()) else {
        return Err(FifoError::NoHoldings { event });
    };
    let mut assigned = Decimal::ZERO;
    for (i, lot) in state.lots.iter_mut().enumerate() {
        lot.quantity = if i == last {
            exact_sub(total, assigned).ok_or(overflow)?
        } else {
            let share = exact_mul(lot.quantity, total).ok_or(overflow)?;
            div_round(share, held, QUANTITY_SCALE).ok_or(overflow)?
        };
        if lot.quantity <= Decimal::ZERO {
            return Err(FifoError::RoundsToZero { event });
        }
        assigned = exact_add(assigned, lot.quantity).ok_or(overflow)?;
    }
    state.quantity = total;
    Ok(())
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

    fn split(ratio: &str) -> Event {
        Event::Split {
            ratio: Ratio::parse(ratio).unwrap(),
        }
    }

    fn stock_dividend(qty: &str) -> Event {
        Event::StockDividend { quantity: q(qty) }
    }

    #[test]
    fn splits_scale_every_lot_and_keep_cost() {
        let result = replay([buy("10", "100"), buy("5", "80"), split("4:1")]).unwrap();

        let lots: Vec<_> = result.lots.iter().map(|l| (l.quantity, l.cost)).collect();
        assert_eq!(lots, [(d("40"), d("100")), (d("20"), d("80"))]);
        assert_eq!(result.quantity(), d("60"));
        assert_eq!(result.cost(), d("180"));
        // Lots keep their acquisition order.
        assert_eq!(result.lots[0].opened_by, 0);
    }

    #[test]
    fn reverse_splits_round_lots_and_put_the_residual_in_the_last_one() {
        // 10 + 10 + 10 = 30 shares at 1:7 → 4.28571428571… ≈ 4.28571429.
        let result = replay([
            buy("10", "70"),
            buy("10", "70"),
            buy("10", "70"),
            split("1:7"),
        ])
        .unwrap();

        let quantities: Vec<_> = result.lots.iter().map(|l| l.quantity).collect();
        assert_eq!(
            quantities,
            [d("1.42857143"), d("1.42857143"), d("1.42857143")]
        );
        assert_eq!(result.quantity(), d("4.28571429"));
        assert_eq!(quantities.iter().sum::<Decimal>(), result.quantity());
        assert_eq!(result.cost(), d("210"));
    }

    #[test]
    fn residuals_keep_lots_summing_to_the_total() {
        // 1 + 2 = 3 shares, 1:3 → exactly 1 share in total.
        let result = replay([buy("1", "10"), buy("2", "20"), split("1:3")]).unwrap();

        let quantities: Vec<_> = result.lots.iter().map(|l| l.quantity).collect();
        assert_eq!(quantities, [d("0.33333333"), d("0.66666667")]);
        assert_eq!(result.quantity(), d("1"));

        // Three lots of 1 at 1:3: rounding each alone would give 0.99999999.
        let result = replay([buy("1", "1"), buy("1", "1"), buy("1", "1"), split("1:3")]).unwrap();
        let quantities: Vec<_> = result.lots.iter().map(|l| l.quantity).collect();
        assert_eq!(
            quantities,
            [d("0.33333333"), d("0.33333333"), d("0.33333334")]
        );
        assert_eq!(quantities.iter().sum::<Decimal>(), d("1"));
        // Selling the whole holding closes every lot.
        let closed = replay([
            buy("1", "1"),
            buy("1", "1"),
            buy("1", "1"),
            split("1:3"),
            sell("1", "5"),
        ])
        .unwrap();
        assert!(closed.lots.is_empty());
        assert_eq!(closed.disposals[0].cost_basis, d("3"));
    }

    #[test]
    fn stock_dividends_spread_over_lots_at_no_cost() {
        // 10 + 30 shares, 4 new shares (10%): each lot grows by 10%.
        let result = replay([buy("10", "100"), buy("30", "600"), stock_dividend("4")]).unwrap();

        let lots: Vec<_> = result.lots.iter().map(|l| (l.quantity, l.cost)).collect();
        assert_eq!(lots, [(d("11"), d("100")), (d("33"), d("600"))]);
        assert_eq!(result.quantity(), d("44"));
        assert_eq!(result.cost(), d("700"));

        // Selling the first (grown) lot takes its whole original cost.
        let sold = replay([
            buy("10", "100"),
            buy("30", "600"),
            stock_dividend("4"),
            sell("11", "130"),
        ])
        .unwrap();
        assert_eq!(sold.disposals[0].cost_basis, d("100"));
        assert_eq!(sold.disposals[0].realized, d("30"));
    }

    #[test]
    fn corporate_actions_need_a_holding() {
        assert_eq!(
            replay([split("2:1")]),
            Err(FifoError::NoHoldings { event: 0 })
        );
        assert_eq!(
            replay([buy("1", "1"), sell("1", "1"), stock_dividend("1")]),
            Err(FifoError::NoHoldings { event: 2 })
        );
    }

    #[test]
    fn a_split_that_rounds_a_lot_to_zero_is_rejected() {
        assert_eq!(
            replay([buy("0.00000001", "1"), split("1:10")]),
            Err(FifoError::RoundsToZero { event: 1 })
        );
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
