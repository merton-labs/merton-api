#![allow(clippy::unwrap_used)]

use merton_api::ledger::decimal::{Amount, Price, Quantity, SignedAmount};
use merton_api::ledger::fifo::{ALLOCATION_SCALE, Disposal, Event, FifoError, Lot, Replay, replay};
use rust_decimal::Decimal;

fn dec(text: &str) -> Decimal {
    text.parse::<Decimal>().unwrap()
}

fn qty(text: &str) -> Quantity {
    Quantity::parse(text).unwrap()
}

fn amt(text: &str) -> Amount {
    Amount::parse(text).unwrap()
}

fn signed(text: &str) -> SignedAmount {
    SignedAmount::parse(text).unwrap()
}

fn acquire(quantity: &str, cost: &str) -> Event {
    Event::Acquire {
        quantity: qty(quantity),
        cost: amt(cost),
    }
}

fn dispose(quantity: &str, proceeds: &str) -> Event {
    Event::Dispose {
        quantity: qty(quantity),
        proceeds: signed(proceeds),
    }
}

fn assert_conserved(events: &[Event], result: &Replay) {
    let acquired_qty: Decimal = events
        .iter()
        .filter_map(|e| match e {
            Event::Acquire { quantity, .. } => Some(quantity.value()),
            _ => None,
        })
        .sum();
    let acquired_cost: Decimal = events
        .iter()
        .filter_map(|e| match e {
            Event::Acquire { cost, .. } => Some(cost.value()),
            _ => None,
        })
        .sum();

    let disposed_qty: Decimal = result.disposals.iter().map(|d| d.quantity).sum();
    let disposed_cost: Decimal = result.disposals.iter().map(|d| d.cost_basis).sum();

    assert_eq!(disposed_qty + result.quantity(), acquired_qty);
    assert_eq!(disposed_cost + result.cost(), acquired_cost);
}

#[test]
fn interleaved_multi_lot_fifo_order_and_realized_gains() {
    // Event 0: Buy 12 @ total cost 300.00 ($25/unit) -> Lot 0 (qty 12, cost 300.00)
    // Event 1: Buy  8 @ total cost 240.00 ($30/unit) -> Lot 1 (qty  8, cost 240.00)
    // Event 2: Sell 5 @ proceeds 160.00:
    //   - Takes 5 of 12 from Lot 0: cost = 300.00 * 5 / 12 = 125.00
    //   - Lot 0 remaining: qty = 7, cost = 300.00 - 125.00 = 175.00
    //   - realized = 160.00 - 125.00 = 35.00
    // Event 3: Buy 10 @ total cost 400.00 ($40/unit) -> Lot 3 (qty 10, cost 400.00)
    // Event 4: Sell 16 @ proceeds 550.00:
    //   - Takes all remaining 7 from Lot 0 (cost = 175.00)
    //   - Takes all 8 from Lot 1 (cost = 240.00)
    //   - Takes 1 of 10 from Lot 3: cost = 400.00 * 1 / 10 = 40.00
    //   - Lot 3 remaining: qty = 9, cost = 400.00 - 40.00 = 360.00
    //   - cost_basis = 175.00 + 240.00 + 40.00 = 455.00
    //   - realized = 550.00 - 455.00 = 95.00
    // Event 5: Buy  6 @ total cost 150.00 ($25/unit) -> Lot 5 (qty  6, cost 150.00)
    // Event 6: Sell 4 @ proceeds 130.00:
    //   - Takes 4 of remaining 9 from Lot 3: cost = 360.00 * 4 / 9 = 160.00
    //   - Lot 3 remaining: qty = 5, cost = 360.00 - 160.00 = 200.00
    //   - realized = 130.00 - 160.00 = -30.00
    let events = vec![
        acquire("12", "300.00"),
        acquire("8", "240.00"),
        dispose("5", "160.00"),
        acquire("10", "400.00"),
        dispose("16", "550.00"),
        acquire("6", "150.00"),
        dispose("4", "130.00"),
    ];

    let result = replay(events.clone()).unwrap();

    assert_eq!(
        result.disposals,
        vec![
            Disposal {
                event: 2,
                quantity: dec("5"),
                cost_basis: dec("125.00"),
                proceeds: dec("160.00"),
                realized: dec("35.00"),
            },
            Disposal {
                event: 4,
                quantity: dec("16"),
                cost_basis: dec("455.00"),
                proceeds: dec("550.00"),
                realized: dec("95.00"),
            },
            Disposal {
                event: 6,
                quantity: dec("4"),
                cost_basis: dec("160.00"),
                proceeds: dec("130.00"),
                realized: dec("-30.00"),
            },
        ]
    );

    assert_eq!(
        result.lots,
        vec![
            Lot {
                opened_by: 3,
                quantity: dec("5"),
                cost: dec("200.00"),
            },
            Lot {
                opened_by: 5,
                quantity: dec("6"),
                cost: dec("150.00"),
            },
        ]
    );

    // Remaining quantity: 5 + 6 = 11; remaining cost: 200.00 + 150.00 = 350.00
    assert_eq!(result.quantity(), dec("11"));
    assert_eq!(result.cost(), dec("350.00"));
    assert_conserved(&events, &result);
}

#[test]
fn conservation_of_cost_and_quantity_across_awkward_sequences() {
    // Sequence 1: Prime lot quantities (7, 11, 13) with non-terminating unit costs,
    // sliced by partial and cross-lot disposals leaving open lots.
    let seq1 = vec![
        acquire("7", "100.00"),
        acquire("11", "250.00"),
        dispose("2", "30.00"),
        dispose("3", "45.00"),
        acquire("13", "400.00"),
        dispose("5", "90.00"),  // consumes last 2 of Lot 0 + 3 of Lot 1
        dispose("4", "85.00"),  // consumes 4 of remaining 8 in Lot 1
        dispose("9", "220.00"), // consumes last 4 of Lot 1 + 5 of Lot 4
        dispose("3", "95.00"),  // consumes 3 of remaining 8 in Lot 4
    ];
    let res1 = replay(seq1.clone()).unwrap();
    // Acquired qty = 7 + 11 + 13 = 31; disposed = 2 + 3 + 5 + 4 + 9 + 3 = 26; open = 5
    assert_eq!(res1.quantity(), dec("5"));
    assert_conserved(&seq1, &res1);

    // Sequence 2: 8-decimal fractional quantities and repeating-decimal allocations.
    let seq2 = vec![
        acquire("0.33333333", "10.00"),
        acquire("0.77777777", "25.50"),
        dispose("0.11111111", "3.50"),
        dispose("0.11111111", "3.60"),
        acquire("0.55555555", "19.99"),
        dispose("0.50000000", "16.00"),
        dispose("0.44444444", "15.25"),
    ];
    let res2 = replay(seq2.clone()).unwrap();
    assert_conserved(&seq2, &res2);

    // Sequence 3: Repeated small partial slices from a single prime lot (17 units @ 1000.00)
    // followed by partial and complete liquidation.
    let mut seq3 = vec![acquire("17", "1000.00")];
    for _ in 0..8 {
        seq3.push(dispose("2", "120.00"));
        let partial = replay(seq3.clone()).unwrap();
        assert_conserved(&seq3, &partial);
    }
    seq3.push(dispose("1", "60.00"));
    let res3 = replay(seq3.clone()).unwrap();
    assert!(res3.lots.is_empty());
    assert_conserved(&seq3, &res3);
}

#[test]
fn partial_allocation_rounding_to_ten_decimal_places() {
    assert_eq!(ALLOCATION_SCALE, 10);

    // Case A: 1 of 3 units from a lot of cost 100, then 1 of remaining 2, then last 1 of 1.
    // - Disposal 0 (1 of 3, cost 100):
    //   100 * 1 / 3 = 33.333333333333... -> rounds to 33.3333333333 (10 dp).
    //   Lot keeps 100 - 33.3333333333 = 66.6666666667 (qty = 2).
    // - Disposal 1 (1 of 2, cost 66.6666666667):
    //   66.6666666667 * 1 / 2 = 33.33333333335 (exact midpoint at 11th decimal place).
    //   10th decimal digit is 3 (odd), so half-to-even rounds to 33.3333333334.
    //   Lot keeps 66.6666666667 - 33.3333333334 = 33.3333333333 (qty = 1).
    // - Disposal 2 (1 of 1, full remaining lot):
    //   Takes exact remaining cost 33.3333333333.
    let three_unit_events = vec![
        acquire("3", "100"),
        dispose("1", "35"),
        dispose("1", "35"),
        dispose("1", "35"),
    ];
    let three_unit_replay = replay(three_unit_events.clone()).unwrap();
    assert_eq!(
        three_unit_replay.disposals[0].cost_basis,
        dec("33.3333333333")
    );
    assert_eq!(
        three_unit_replay.disposals[1].cost_basis,
        dec("33.3333333334")
    );
    assert_eq!(
        three_unit_replay.disposals[2].cost_basis,
        dec("33.3333333333")
    );
    assert_conserved(&three_unit_events, &three_unit_replay);

    // Case B: 2 of 7 units from a lot of cost 10.
    // - Exact: 10 * 2 / 7 = 20 / 7 = 2.857142857142857...
    // - 10th decimal digit is 1, 11th is 4 (< 5) -> rounds to 2.8571428571.
    // - Remaining lot: qty = 7 - 2 = 5, cost = 10 - 2.8571428571 = 7.1428571429.
    // - Realized on proceeds 3.00: 3.00 - 2.8571428571 = 0.1428571429.
    let seven_unit_events = vec![acquire("7", "10"), dispose("2", "3.00")];
    let seven_unit_replay = replay(seven_unit_events.clone()).unwrap();
    assert_eq!(
        seven_unit_replay.disposals[0].cost_basis,
        dec("2.8571428571")
    );
    assert_eq!(seven_unit_replay.disposals[0].realized, dec("0.1428571429"));
    assert_eq!(
        seven_unit_replay.lots,
        vec![Lot {
            opened_by: 0,
            quantity: dec("5"),
            cost: dec("7.1428571429"),
        }]
    );
    assert_conserved(&seven_unit_events, &seven_unit_replay);
}

#[test]
fn half_to_even_tie_breaking_at_tenth_decimal_place() {
    // Construct two direct midpoint ties at the 11th decimal digit (5 * 10^-11):
    //
    // Tie 1 (rounds down to even):
    //   Lot: quantity = 200, cost = 0.00000003. Dispose taken = 3.
    //   Exact cost = 0.00000003 * 3 / 200 = 0.00000009 / 200 = 0.00000000045.
    //   The 10th decimal digit is 4 (even) and the 11th is 5 (exact half-way tie).
    //   Half-to-even rounds to 0.0000000004.
    //   Remaining lot cost = 0.00000003 - 0.0000000004 = 0.0000000296.
    let tie_even = replay(vec![acquire("200", "0.00000003"), dispose("3", "1.00")]).unwrap();
    assert_eq!(tie_even.disposals[0].cost_basis, dec("0.0000000004"));
    assert_eq!(tie_even.lots[0].cost, dec("0.0000000296"));

    // Tie 2 (rounds up to even):
    //   Lot: quantity = 200, cost = 0.00000001. Dispose taken = 3.
    //   Exact cost = 0.00000001 * 3 / 200 = 0.00000003 / 200 = 0.00000000015.
    //   The 10th decimal digit is 1 (odd) and the 11th is 5 (exact half-way tie).
    //   Half-to-even rounds to 0.0000000002.
    //   Remaining lot cost = 0.00000001 - 0.0000000002 = 0.0000000098.
    let tie_odd = replay(vec![acquire("200", "0.00000001"), dispose("3", "1.00")]).unwrap();
    assert_eq!(tie_odd.disposals[0].cost_basis, dec("0.0000000002"));
    assert_eq!(tie_odd.lots[0].cost, dec("0.0000000098"));
}

#[test]
fn selling_full_position_empties_lots_and_allocates_exact_remaining_cost() {
    // Event 0: Buy 3 @ 100.00
    // Event 1: Buy 7 @ 50.00
    // Event 2: Sell 1 @ 40.00 -> consumes 1 of 3 from Lot 0:
    //   cost_basis = 100.00 * 1 / 3 = 33.3333333333
    //   Lot 0 remaining: qty = 2, cost = 66.6666666667
    // Event 3: Sell remaining 9 (2 from Lot 0 + 7 from Lot 1) @ 120.00:
    //   Takes exact remaining cost of Lot 0 (66.6666666667) + full Lot 1 (50.00)
    //   cost_basis = 116.6666666667
    //   realized = 120.00 - 116.6666666667 = 3.3333333333
    let events = vec![
        acquire("3", "100.00"),
        acquire("7", "50.00"),
        dispose("1", "40.00"),
        dispose("9", "120.00"),
    ];
    let result = replay(events.clone()).unwrap();

    assert!(result.lots.is_empty());
    assert_eq!(result.quantity(), dec("0"));
    assert_eq!(result.cost(), dec("0"));
    assert_eq!(result.disposals[1].cost_basis, dec("116.6666666667"));
    assert_eq!(result.disposals[1].realized, dec("3.3333333333"));
    assert_conserved(&events, &result);
}

#[test]
fn insufficient_holdings_reports_exact_event_available_and_requested() {
    // Selling as the very first event (index 0) when available is 0.
    let err_first = replay(vec![dispose("1.50", "10.00")]).unwrap_err();
    assert_eq!(
        err_first,
        FifoError::InsufficientHoldings {
            event: 0,
            available: dec("0"),
            requested: dec("1.50"),
        }
    );

    // Exceeding available holdings by 0.00000001 on event index 3:
    //   Event 0: Buy 10.00000000
    //   Event 1: Sell 3.50000000 -> available = 6.50000000
    //   Event 2: Buy  2.25000000 -> available = 8.75000000
    //   Event 3: Sell 8.75000001 -> exceeds 8.75000000 by 0.00000001
    let err_tiny = replay(vec![
        acquire("10.00000000", "100.00"),
        dispose("3.50000000", "40.00"),
        acquire("2.25000000", "25.00"),
        dispose("8.75000001", "90.00"),
    ])
    .unwrap_err();
    assert_eq!(
        err_tiny,
        FifoError::InsufficientHoldings {
            event: 3,
            available: dec("8.75000000"),
            requested: dec("8.75000001"),
        }
    );
}

#[test]
fn zero_cost_lots_zero_proceeds_and_negative_proceeds_losses() {
    // Event 0: Acquire 50 units at 0.00 cost (e.g., bonus issue)
    // Event 1: Dispose 20 units for 120.00 proceeds -> cost_basis = 0, realized = 120.00
    // Event 2: Dispose 30 units for 0.00 proceeds   -> cost_basis = 0, realized = 0.00
    // Event 3: Acquire 10 units at 250.00 cost ($25/unit)
    // Event 4: Dispose 4 units for 0.00 proceeds    -> cost_basis = 100.00, realized = -100.00
    // Event 5: Dispose 6 units for -15.75 proceeds  -> cost_basis = 150.00, realized = -165.75
    let events = vec![
        acquire("50", "0.00"),
        dispose("20", "120.00"),
        dispose("30", "0.00"),
        acquire("10", "250.00"),
        dispose("4", "0.00"),
        dispose("6", "-15.75"),
    ];
    let result = replay(events.clone()).unwrap();

    assert!(result.lots.is_empty());
    assert_eq!(
        result.disposals,
        vec![
            Disposal {
                event: 1,
                quantity: dec("20"),
                cost_basis: dec("0"),
                proceeds: dec("120.00"),
                realized: dec("120.00"),
            },
            Disposal {
                event: 2,
                quantity: dec("30"),
                cost_basis: dec("0"),
                proceeds: dec("0.00"),
                realized: dec("0.00"),
            },
            Disposal {
                event: 4,
                quantity: dec("4"),
                cost_basis: dec("100.00"),
                proceeds: dec("0.00"),
                realized: dec("-100.00"),
            },
            Disposal {
                event: 5,
                quantity: dec("6"),
                cost_basis: dec("150.00"),
                proceeds: dec("-15.75"),
                realized: dec("-165.75"),
            },
        ]
    );
    assert_conserved(&events, &result);
}

#[test]
fn eight_decimal_fractional_quantities() {
    // Event 0: Buy 0.12345678 units @ cost 1234.56780000 ($10,000/unit)
    // Event 1: Buy 0.00000005 units @ cost 0.00060000 ($12,000/unit)
    // Event 2: Sell 0.02345678 units @ proceeds 250.00000000:
    //   - Takes 0.02345678 of 0.12345678 from Lot 0:
    //     cost_basis = 1234.56780000 * 0.02345678 / 0.12345678 = 234.56780000
    //   - Lot 0 remaining: qty = 0.10000000, cost = 1000.00000000
    //   - realized = 250.00000000 - 234.56780000 = 15.43220000
    // Event 3: Sell 0.10000003 units @ proceeds 1000.00050000:
    //   - Takes remaining 0.10000000 of Lot 0 (cost = 1000.00000000)
    //   - Takes 0.00000003 of 0.00000005 from Lot 1:
    //     cost = 0.00060000 * 0.00000003 / 0.00000005 = 0.00036000
    //   - Total cost_basis = 1000.00036000
    //   - realized = 1000.00050000 - 1000.00036000 = 0.00014000
    //   - Lot 1 remaining: qty = 0.00000002, cost = 0.00024000
    let events = vec![
        acquire("0.12345678", "1234.56780000"),
        acquire("0.00000005", "0.00060000"),
        dispose("0.02345678", "250.00000000"),
        dispose("0.10000003", "1000.00050000"),
    ];
    let result = replay(events.clone()).unwrap();

    assert_eq!(
        result.disposals,
        vec![
            Disposal {
                event: 2,
                quantity: dec("0.02345678"),
                cost_basis: dec("234.56780000"),
                proceeds: dec("250.00000000"),
                realized: dec("15.43220000"),
            },
            Disposal {
                event: 3,
                quantity: dec("0.10000003"),
                cost_basis: dec("1000.00036000"),
                proceeds: dec("1000.00050000"),
                realized: dec("0.00014000"),
            },
        ]
    );
    assert_eq!(
        result.lots,
        vec![Lot {
            opened_by: 1,
            quantity: dec("0.00000002"),
            cost: dec("0.00024000"),
        }]
    );
    assert_conserved(&events, &result);
}

#[test]
fn boundary_large_values_do_not_panic() {
    // Maximum allowed input has 15 integer digits and 8 fractional digits:
    // 999999999999999.99999999.
    // Note that cost * taken for two ~10^15 numbers is ~10^30, which exceeds
    // Decimal::MAX (~7.92 * 10^28); the implementation must either succeed
    // (by computing ratio when multiplication overflows) or return FifoError::Overflow
    // without panicking.
    let events = vec![
        acquire("999999999999999.99999999", "999999999999999.99999999"),
        acquire("999999999999999.99999999", "999999999999999.99999999"),
        dispose("500000000000000.00000000", "600000000000000.00000000"),
        dispose("999999999999999.99999999", "-999999999999999.99999999"),
        dispose("499999999999999.99999999", "999999999999999.99999999"),
    ];

    match replay(events.clone()) {
        Ok(result) => {
            assert!(result.lots.is_empty());
            assert_conserved(&events, &result);
        }
        Err(FifoError::Overflow { event }) => {
            assert!(event < events.len());
        }
        Err(other) => panic!("unexpected error on boundary values: {other:?}"),
    }
}

#[test]
fn decimal_sign_rules_scale_preservation_and_negative_zero() {
    // Quantity must be strictly > 0.
    for zero_or_neg in [
        "0",
        "0.00",
        "-0",
        "-0.00000000",
        "-0.00000001",
        "-1",
        "-100.50",
    ] {
        assert!(
            Quantity::parse(zero_or_neg).is_err(),
            "Quantity should reject {zero_or_neg:?}"
        );
    }
    let q = Quantity::parse("  0.00000001\n").unwrap();
    assert_eq!(q.value(), dec("0.00000001"));
    assert_eq!(q.to_string(), "0.00000001");

    // Price and Amount must be >= 0 (accept 0 and -0, reject negatives).
    for valid_zero in ["0", "0.00", "-0", "-0.00", "  -0.00000000  "] {
        let p = Price::parse(valid_zero).unwrap();
        let a = Amount::parse(valid_zero).unwrap();
        assert_eq!(p.value(), Decimal::ZERO);
        assert!(!p.value().is_sign_negative());
        assert_eq!(a.value(), Decimal::ZERO);
        assert!(!a.value().is_sign_negative());
    }
    for neg in ["-0.00000001", "-1", "-100.50"] {
        assert!(Price::parse(neg).is_err(), "Price should reject {neg:?}");
        assert!(Amount::parse(neg).is_err(), "Amount should reject {neg:?}");
    }

    // SignedAmount accepts positive, zero, -0, and negative values.
    let s_neg_zero = SignedAmount::parse("-0.00").unwrap();
    assert_eq!(s_neg_zero.value(), Decimal::ZERO);
    assert!(!s_neg_zero.value().is_sign_negative());
    assert_eq!(s_neg_zero.to_string(), "0.00");

    let s_neg = SignedAmount::parse(" -100.50 ").unwrap();
    assert_eq!(s_neg.value(), dec("-100.50"));
    assert_eq!(s_neg.to_string(), "-100.50");

    // Display preserves entered scale across types.
    assert_eq!(Amount::parse("100.50").unwrap().to_string(), "100.50");
    assert_eq!(Amount::parse("100").unwrap().to_string(), "100");
    assert_eq!(Price::parse("42.12300").unwrap().to_string(), "42.12300");
    assert_eq!(
        Quantity::parse("1.00000000").unwrap().to_string(),
        "1.00000000"
    );
}

#[test]
fn decimal_grammar_and_precision_limits() {
    // Precision limits: up to 15 significant integer digits and up to 8 fractional digits.
    let max_pos = "999999999999999.99999999";
    let max_neg = "-999999999999999.99999999";
    assert_eq!(Quantity::parse(max_pos).unwrap().to_string(), max_pos);
    assert_eq!(SignedAmount::parse(max_neg).unwrap().to_string(), max_neg);

    // Leading zeros before 15 significant integer digits are allowed.
    assert_eq!(
        Amount::parse("000999999999999999.12345678")
            .unwrap()
            .value(),
        dec("999999999999999.12345678")
    );

    // 16 significant digits before the decimal point must be rejected.
    for too_many_int in [
        "1000000000000000",
        "1000000000000000.0",
        "-1000000000000000",
        "01000000000000000",
    ] {
        assert!(
            SignedAmount::parse(too_many_int).is_err(),
            "should reject >15 integer digits: {too_many_int:?}"
        );
    }

    // 9 digits after the decimal point must be rejected.
    for too_many_frac in ["1.123456789", "0.000000001", "1.000000000", "-1.123456789"] {
        assert!(
            SignedAmount::parse(too_many_frac).is_err(),
            "should reject >8 fractional digits: {too_many_frac:?}"
        );
    }

    // Invalid grammar forms must all be rejected.
    let invalid_inputs = [
        "", "   ", "-", "+", "+10", "+0", "+12.34", ".5", "-.5", "5.", "-5.", ".", "1e5", "1E5",
        "1.5e-2", "1_000", "10.00_00", "_10", "10_", "1,000", "12,34", "1 000", "- 5", "12 .34",
        "12. 34", "1.2.3", "--5", "+-5", "5-", "12a", "NaN", "inf",
    ];
    for input in invalid_inputs {
        assert!(
            SignedAmount::parse(input).is_err(),
            "SignedAmount should reject invalid grammar {input:?}"
        );
        assert!(
            Amount::parse(input).is_err(),
            "Amount should reject invalid grammar {input:?}"
        );
        assert!(
            Price::parse(input).is_err(),
            "Price should reject invalid grammar {input:?}"
        );
        assert!(
            Quantity::parse(input).is_err(),
            "Quantity should reject invalid grammar {input:?}"
        );
    }
}
