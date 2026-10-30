//! On-demand valuation at an explicit `as_of` date (ADR-VAL-01 … VAL-06).
//!
//! This module is pure: the service loads holdings, cash balances, prices
//! and FX rates on or before `as_of`, and the functions here value them.
//!
//! Rounding (ADR-NUM-05, P5): quantity × price and the FX multiplication
//! are exact, and each base-currency value is rounded half-to-even to
//! [`VALUE_SCALE`] places once, at the end. Dividing by an inverse rate
//! rounds the same way. Totals add those rounded values exactly, so the
//! items always sum to the totals. Allocation percentages are rounded to
//! [`PERCENT_SCALE`] places and are not forced to sum to 100.
//!
//! An asset without a price or rate is never valued at zero: it is left
//! out of the totals, listed with a reason, and the result is partial
//! (ADR-VAL-04).

pub mod service;

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use sea_orm::DbErr;
use time::Date;
use uuid::Uuid;

use crate::auth::FieldError;
use crate::instruments::AssetClass;
use crate::ledger::decimal::{exact_add, exact_mul, mul_round, signed_div_round};
use crate::market_data::Source;

/// Places for base-currency values.
pub const VALUE_SCALE: u32 = 8;
/// Places for allocation percentages.
pub const PERCENT_SCALE: u32 = 4;
/// A price older than this many days before `as_of` is stale (ADR-VAL-02).
pub const PRICE_STALE_DAYS: i64 = 7;
/// An FX rate older than this many days before `as_of` is stale.
pub const FX_STALE_DAYS: i64 = 3;

/// An open position at `as_of`, from FIFO replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Position {
    pub instrument_id: Uuid,
    pub name: String,
    pub symbol: Option<String>,
    pub asset_class: AssetClass,
    pub quote_currency: String,
    pub quantity: Decimal,
    /// Cost of the open lots, in the quote currency (ADR-LEDG-10).
    pub cost_basis: Decimal,
}

/// A cash account's balance at `as_of`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CashBalance {
    pub cash_account_id: Uuid,
    pub name: String,
    pub currency: String,
    pub balance: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holdings {
    pub portfolio_id: Uuid,
    pub as_of: Date,
    pub base_currency: String,
    pub positions: Vec<Position>,
    pub cash: Vec<CashBalance>,
}

/// The latest price on or before `as_of`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PricePoint {
    pub price: Decimal,
    pub date: Date,
    pub source: Source,
}

/// A stored FX rate: one `base` is worth `rate` `quote`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredRate {
    pub rate: Decimal,
    pub date: Date,
    pub source: Source,
}

/// The rate used to convert one currency into the portfolio's base
/// currency. `inverse` means the stored pair was base/currency, so values
/// are divided by `rate` instead of multiplied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FxPoint {
    pub rate: Decimal,
    pub date: Date,
    pub source: Source,
    pub inverse: bool,
}

/// Picks between the direct rate (currency/base) and the inverse rate
/// (base/currency), both the latest on or before `as_of`: the later date
/// wins, and the direct rate wins a tie (ADR-VAL-06).
pub fn choose_rate(direct: Option<StoredRate>, inverse: Option<StoredRate>) -> Option<FxPoint> {
    let as_point = |r: StoredRate, inverse| FxPoint {
        rate: r.rate,
        date: r.date,
        source: r.source,
        inverse,
    };
    match (direct, inverse) {
        (Some(d), Some(i)) if i.date > d.date => Some(as_point(i, true)),
        (Some(d), _) => Some(as_point(d, false)),
        (None, Some(i)) => Some(as_point(i, true)),
        (None, None) => None,
    }
}

/// Market data loaded for a valuation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarketData {
    pub prices: BTreeMap<Uuid, PricePoint>,
    /// By currency; only currencies other than the base currency.
    pub rates: BTreeMap<String, FxPoint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnvaluedReason {
    MissingPrice,
    MissingFxRate,
    /// The exact value does not fit the supported numeric range.
    OutOfRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PriceQuote {
    pub price: Decimal,
    pub date: Date,
    pub source: Source,
    pub stale: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FxQuote {
    pub rate: Decimal,
    pub date: Date,
    pub source: Source,
    pub inverse: bool,
    pub stale: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValuedPosition {
    pub position: Position,
    pub price: Option<PriceQuote>,
    /// quantity × price in the quote currency, exact.
    pub market_value: Option<Decimal>,
    /// `None` when the quote currency is the base currency.
    pub fx: Option<FxQuote>,
    /// In the base currency, rounded to [`VALUE_SCALE`].
    pub value: Option<Decimal>,
    pub unvalued: Option<UnvaluedReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValuedCash {
    pub cash: CashBalance,
    pub fx: Option<FxQuote>,
    pub value: Option<Decimal>,
    pub unvalued: Option<UnvaluedReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Valuation {
    pub portfolio_id: Uuid,
    pub as_of: Date,
    pub base_currency: String,
    pub positions: Vec<ValuedPosition>,
    pub cash: Vec<ValuedCash>,
    /// Sum of the valued positions.
    pub positions_value: Decimal,
    /// Sum of the valued cash balances (may be negative).
    pub cash_value: Decimal,
    /// Net Portfolio Value: positions plus cash.
    pub net_value: Decimal,
    /// Some asset could not be valued and is missing from the totals.
    pub partial: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AllocationClass {
    Stock,
    Etf,
    MutualFund,
    Cash,
}

impl From<AssetClass> for AllocationClass {
    fn from(class: AssetClass) -> Self {
        match class {
            AssetClass::Stock => Self::Stock,
            AssetClass::Etf => Self::Etf,
            AssetClass::MutualFund => Self::MutualFund,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassShare {
    pub class: AllocationClass,
    pub value: Decimal,
    /// Signed share of the Net Portfolio Value; `None` when it is ≤ 0.
    pub percent: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Allocation {
    pub portfolio_id: Uuid,
    pub as_of: Date,
    pub base_currency: String,
    pub net_value: Decimal,
    pub classes: Vec<ClassShare>,
    /// Some cash account is overdrawn. Not a margin loan (ADR-VAL-05).
    pub negative_cash: bool,
    pub partial: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ValuationError {
    #[error("validation failed")]
    Validation(Vec<FieldError>),
    #[error("portfolio not found")]
    NotFound,
    /// A total is too large to represent exactly.
    #[error("the portfolio's value is out of range")]
    OutOfRange,
    #[error("database error: {0}")]
    Database(#[from] DbErr),
}

fn is_stale(as_of: Date, date: Date, days: i64) -> bool {
    (as_of - date).whole_days() > days
}

fn fx_quote(as_of: Date, point: FxPoint) -> FxQuote {
    FxQuote {
        rate: point.rate,
        date: point.date,
        source: point.source,
        inverse: point.inverse,
        stale: is_stale(as_of, point.date, FX_STALE_DAYS),
    }
}

/// Converts an exact amount in `currency` to the base currency.
fn to_base(
    amount: Decimal,
    currency: &str,
    base_currency: &str,
    as_of: Date,
    rates: &BTreeMap<String, FxPoint>,
) -> (Option<FxQuote>, Result<Decimal, UnvaluedReason>) {
    if currency == base_currency {
        let value = mul_round(amount, Decimal::ONE, VALUE_SCALE).ok_or(UnvaluedReason::OutOfRange);
        return (None, value);
    }
    let Some(point) = rates.get(currency).copied() else {
        return (None, Err(UnvaluedReason::MissingFxRate));
    };
    let value = if point.inverse {
        signed_div_round(amount, point.rate, VALUE_SCALE)
    } else {
        mul_round(amount, point.rate, VALUE_SCALE)
    };
    (
        Some(fx_quote(as_of, point)),
        value.ok_or(UnvaluedReason::OutOfRange),
    )
}

fn value_position(
    position: Position,
    base_currency: &str,
    as_of: Date,
    market: &MarketData,
) -> ValuedPosition {
    let Some(point) = market.prices.get(&position.instrument_id).copied() else {
        return ValuedPosition {
            position,
            price: None,
            market_value: None,
            fx: None,
            value: None,
            unvalued: Some(UnvaluedReason::MissingPrice),
        };
    };
    let price = Some(PriceQuote {
        price: point.price,
        date: point.date,
        source: point.source,
        stale: is_stale(as_of, point.date, PRICE_STALE_DAYS),
    });
    let Some(market_value) = exact_mul(position.quantity, point.price) else {
        return ValuedPosition {
            position,
            price,
            market_value: None,
            fx: None,
            value: None,
            unvalued: Some(UnvaluedReason::OutOfRange),
        };
    };
    let (fx, value) = to_base(
        market_value,
        &position.quote_currency,
        base_currency,
        as_of,
        &market.rates,
    );
    ValuedPosition {
        position,
        price,
        market_value: Some(market_value),
        fx,
        value: value.ok(),
        unvalued: value.err(),
    }
}

fn value_cash(
    cash: CashBalance,
    base_currency: &str,
    as_of: Date,
    market: &MarketData,
) -> ValuedCash {
    // A zero balance is worth exactly zero in any currency.
    let (fx, value) = if cash.balance.is_zero() {
        (None, Ok(Decimal::new(0, VALUE_SCALE)))
    } else {
        to_base(
            cash.balance,
            &cash.currency,
            base_currency,
            as_of,
            &market.rates,
        )
    };
    ValuedCash {
        cash,
        fx,
        value: value.ok(),
        unvalued: value.err(),
    }
}

fn sum(values: impl IntoIterator<Item = Decimal>) -> Result<Decimal, ValuationError> {
    values
        .into_iter()
        .try_fold(Decimal::new(0, VALUE_SCALE), exact_add)
        .ok_or(ValuationError::OutOfRange)
}

/// Values `holdings` with the market data on or before `as_of`.
pub fn value(holdings: Holdings, market: &MarketData) -> Result<Valuation, ValuationError> {
    let Holdings {
        portfolio_id,
        as_of,
        base_currency,
        positions,
        cash,
    } = holdings;
    let positions: Vec<_> = positions
        .into_iter()
        .map(|p| value_position(p, &base_currency, as_of, market))
        .collect();
    let cash: Vec<_> = cash
        .into_iter()
        .map(|c| value_cash(c, &base_currency, as_of, market))
        .collect();

    let positions_value = sum(positions.iter().filter_map(|p| p.value))?;
    let cash_value = sum(cash.iter().filter_map(|c| c.value))?;
    let net_value = exact_add(positions_value, cash_value).ok_or(ValuationError::OutOfRange)?;
    let partial =
        positions.iter().any(|p| p.unvalued.is_some()) || cash.iter().any(|c| c.unvalued.is_some());
    Ok(Valuation {
        portfolio_id,
        as_of,
        base_currency,
        positions,
        cash,
        positions_value,
        cash_value,
        net_value,
        partial,
    })
}

/// Signed allocation by asset class on Net Portfolio Value (ADR-VAL-05).
/// Only valued assets count; a partial valuation gives a partial
/// allocation.
pub fn allocate(valuation: &Valuation) -> Result<Allocation, ValuationError> {
    let mut by_class: BTreeMap<AllocationClass, Vec<Decimal>> = BTreeMap::new();
    for p in &valuation.positions {
        if let Some(value) = p.value {
            by_class
                .entry(p.position.asset_class.into())
                .or_default()
                .push(value);
        }
    }
    for c in &valuation.cash {
        if let Some(value) = c.value {
            by_class
                .entry(AllocationClass::Cash)
                .or_default()
                .push(value);
        }
    }

    let npv = valuation.net_value;
    let hundred = Decimal::ONE_HUNDRED;
    let classes = by_class
        .into_iter()
        .map(|(class, values)| {
            let value = sum(values)?;
            let percent = if npv > Decimal::ZERO {
                let scaled = exact_mul(value, hundred).ok_or(ValuationError::OutOfRange)?;
                Some(
                    signed_div_round(scaled, npv, PERCENT_SCALE)
                        .ok_or(ValuationError::OutOfRange)?,
                )
            } else {
                None
            };
            Ok(ClassShare {
                class,
                value,
                percent,
            })
        })
        .collect::<Result<Vec<_>, ValuationError>>()?;

    Ok(Allocation {
        portfolio_id: valuation.portfolio_id,
        as_of: valuation.as_of,
        base_currency: valuation.base_currency.clone(),
        net_value: npv,
        classes,
        negative_cash: valuation
            .cash
            .iter()
            .any(|c| c.cash.balance < Decimal::ZERO),
        partial: valuation.partial,
    })
}

#[cfg(test)]
mod tests {
    use time::macros::date;

    use super::*;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    const AS_OF: Date = date!(2026 - 01 - 15);

    fn position(class: AssetClass, currency: &str, quantity: &str) -> Position {
        Position {
            instrument_id: Uuid::now_v7(),
            name: "X".to_owned(),
            symbol: None,
            asset_class: class,
            quote_currency: currency.to_owned(),
            quantity: d(quantity),
            cost_basis: d("0"),
        }
    }

    fn cash(currency: &str, balance: &str) -> CashBalance {
        CashBalance {
            cash_account_id: Uuid::now_v7(),
            name: "Cash".to_owned(),
            currency: currency.to_owned(),
            balance: d(balance),
        }
    }

    fn holdings(base: &str, positions: Vec<Position>, cash: Vec<CashBalance>) -> Holdings {
        Holdings {
            portfolio_id: Uuid::now_v7(),
            as_of: AS_OF,
            base_currency: base.to_owned(),
            positions,
            cash,
        }
    }

    fn price(value: &str, date: Date) -> PricePoint {
        PricePoint {
            price: d(value),
            date,
            source: Source::Manual,
        }
    }

    fn stored(rate: &str, date: Date) -> StoredRate {
        StoredRate {
            rate: d(rate),
            date,
            source: Source::Manual,
        }
    }

    fn rate(value: &str, date: Date, inverse: bool) -> FxPoint {
        FxPoint {
            rate: d(value),
            date,
            source: Source::Manual,
            inverse,
        }
    }

    #[test]
    fn the_later_rate_wins_and_direct_wins_a_tie() {
        let early = date!(2026 - 01 - 10);
        let late = date!(2026 - 01 - 14);
        let direct = |date| Some(stored("36", date));
        let inverse = |date| Some(stored("0.0277", date));

        let pick = choose_rate(direct(early), inverse(late)).unwrap();
        assert!(pick.inverse);
        assert_eq!(pick.date, late);
        let pick = choose_rate(direct(late), inverse(early)).unwrap();
        assert!(!pick.inverse);
        let pick = choose_rate(direct(late), inverse(late)).unwrap();
        assert!(!pick.inverse, "direct wins a tie");
        assert!(choose_rate(None, inverse(early)).unwrap().inverse);
        assert_eq!(choose_rate(None, None), None);
    }

    #[test]
    fn values_convert_exactly_and_round_once() {
        let stock = position(AssetClass::Stock, "USD", "1000.12345678");
        let id = stock.instrument_id;
        let mut market = MarketData::default();
        market.prices.insert(id, price("1", AS_OF));
        market
            .rates
            .insert("USD".to_owned(), rate("36.12345678", AS_OF, false));
        let valuation = value(holdings("THB", vec![stock], vec![]), &market).unwrap();
        let p = &valuation.positions[0];
        assert_eq!(p.market_value, Some(d("1000.12345678")));
        // 36127.916465655... rounded once, not after each step.
        assert_eq!(p.value, Some(d("36127.91646566")));
        assert_eq!(valuation.net_value, d("36127.91646566"));
        assert!(!valuation.partial);
    }

    #[test]
    fn inverse_rates_divide_and_same_currency_needs_no_rate() {
        let usd = cash("USD", "-100");
        let thb = cash("THB", "50.5");
        let mut market = MarketData::default();
        market
            .rates
            .insert("USD".to_owned(), rate("0.03", AS_OF, true));
        let valuation = value(holdings("THB", vec![], vec![usd, thb]), &market).unwrap();
        assert_eq!(valuation.cash[0].value, Some(d("-3333.33333333")));
        assert!(valuation.cash[0].fx.unwrap().inverse);
        assert_eq!(valuation.cash[1].value, Some(d("50.50000000")));
        assert_eq!(valuation.cash[1].fx, None);
        assert_eq!(valuation.cash_value, d("-3282.83333333"));
    }

    #[test]
    fn missing_data_is_listed_not_valued_at_zero() {
        let priced = position(AssetClass::Stock, "THB", "10");
        let unpriced = position(AssetClass::Etf, "THB", "5");
        let foreign = position(AssetClass::Stock, "USD", "1");
        let mut market = MarketData::default();
        market
            .prices
            .insert(priced.instrument_id, price("2", AS_OF));
        market
            .prices
            .insert(foreign.instrument_id, price("100", AS_OF));
        let empty_usd = cash("USD", "0");
        let usd = cash("USD", "5");

        let valuation = value(
            holdings("THB", vec![priced, unpriced, foreign], vec![empty_usd, usd]),
            &market,
        )
        .unwrap();
        assert!(valuation.partial);
        let reasons: Vec<_> = valuation.positions.iter().map(|p| p.unvalued).collect();
        assert_eq!(
            reasons,
            [
                None,
                Some(UnvaluedReason::MissingPrice),
                Some(UnvaluedReason::MissingFxRate)
            ]
        );
        assert_eq!(valuation.positions[2].market_value, Some(d("100")));
        assert_eq!(valuation.cash[0].value, Some(d("0")), "zero needs no rate");
        assert_eq!(
            valuation.cash[1].unvalued,
            Some(UnvaluedReason::MissingFxRate)
        );
        assert_eq!(valuation.positions_value, d("20"));
        assert_eq!(valuation.net_value, d("20"));
    }

    #[test]
    fn staleness_follows_the_owner_thresholds() {
        let stock = position(AssetClass::Stock, "USD", "1");
        let id = stock.instrument_id;
        let mut market = MarketData::default();
        market.prices.insert(id, price("1", date!(2026 - 01 - 08)));
        market
            .rates
            .insert("USD".to_owned(), rate("36", date!(2026 - 01 - 12), false));
        let fresh = value(holdings("THB", vec![stock.clone()], vec![]), &market).unwrap();
        assert!(
            !fresh.positions[0].price.unwrap().stale,
            "7 days is not stale"
        );
        assert!(!fresh.positions[0].fx.unwrap().stale, "3 days is not stale");

        market.prices.insert(id, price("1", date!(2026 - 01 - 07)));
        market
            .rates
            .insert("USD".to_owned(), rate("36", date!(2026 - 01 - 11), false));
        let stale = value(holdings("THB", vec![stock], vec![]), &market).unwrap();
        assert!(stale.positions[0].price.unwrap().stale);
        assert!(stale.positions[0].fx.unwrap().stale);
        assert!(!stale.partial, "stale values are still used");
    }

    #[test]
    fn allocation_is_signed_on_net_value() {
        let stock = position(AssetClass::Stock, "USD", "100");
        let mut market = MarketData::default();
        market
            .prices
            .insert(stock.instrument_id, price("100", AS_OF));
        let valuation = value(
            holdings("USD", vec![stock], vec![cash("USD", "-2000")]),
            &market,
        )
        .unwrap();
        let allocation = allocate(&valuation).unwrap();
        assert_eq!(allocation.net_value, d("8000"));
        assert!(allocation.negative_cash);
        let shares: Vec<_> = allocation
            .classes
            .iter()
            .map(|c| (c.class, c.percent))
            .collect();
        assert_eq!(
            shares,
            [
                (AllocationClass::Stock, Some(d("125.0000"))),
                (AllocationClass::Cash, Some(d("-25.0000")))
            ]
        );
    }

    #[test]
    fn allocation_percentages_are_null_without_positive_net_value() {
        let stock = position(AssetClass::Stock, "USD", "10");
        let mut market = MarketData::default();
        market
            .prices
            .insert(stock.instrument_id, price("100", AS_OF));
        for balance in ["-1000", "-1500"] {
            let valuation = value(
                holdings("USD", vec![stock.clone()], vec![cash("USD", balance)]),
                &market,
            )
            .unwrap();
            let allocation = allocate(&valuation).unwrap();
            assert!(allocation.net_value <= Decimal::ZERO);
            assert!(allocation.classes.iter().all(|c| c.percent.is_none()));
        }
    }

    #[test]
    fn percentages_round_to_four_places_without_forcing_one_hundred() {
        let a = position(AssetClass::Stock, "USD", "1");
        let b = position(AssetClass::Etf, "USD", "1");
        let c = position(AssetClass::MutualFund, "USD", "1");
        let mut market = MarketData::default();
        for p in [&a, &b, &c] {
            market.prices.insert(p.instrument_id, price("1", AS_OF));
        }
        let valuation = value(holdings("USD", vec![a, b, c], vec![]), &market).unwrap();
        let allocation = allocate(&valuation).unwrap();
        let percents: Vec<_> = allocation.classes.iter().map(|c| c.percent).collect();
        assert_eq!(percents, [Some(d("33.3333")); 3]);
        assert!(allocation.classes.iter().all(|c| c.value == d("1")));
    }
}
