//! Returns per holding, as in Snowball Analytics' Holdings table
//! (ADR-RET-01): capital gain, dividends, realized P&L, fees, taxes and
//! total profit, each in the instrument's quote currency.
//!
//! This module is pure: the service replays each instrument's history up to
//! `as_of`, adds up its transactions, and loads its latest price.
//!
//! - **Capital gain** = market value − cost basis of the open lots.
//! - **Total profit** = capital gain + realized P&L + dividends received
//!   (after their tax and fee) − fees − taxes. Fees are the fees on buys and
//!   sells plus `FEE` and `OTHER_EXPENSE` entries linked to the instrument;
//!   taxes are the taxes on buys and sells plus `TAX` entries, less
//!   `TAX_RETURN` entries.
//! - **Percentages:** capital gain over cost basis, and total profit over
//!   the cost of every purchase (`total_bought`), both × 100 and rounded
//!   half-to-even to [`PERCENT_SCALE`] places; `None` when the divisor is
//!   not positive. Average cost is cost basis ÷ quantity, rounded to
//!   [`AVERAGE_SCALE`] places. Every other figure is exact.
//! - An open position without a price has no market value, capital gain or
//!   total profit; it is never valued at zero, and the result is partial.

pub mod service;

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use sea_orm::DbErr;
use time::Date;
use uuid::Uuid;

use crate::auth::FieldError;
use crate::instruments::AssetClass;
use crate::ledger::decimal::{div_round, exact_add, exact_mul, exact_sub, signed_div_round};
use crate::valuation::{PERCENT_SCALE, PricePoint, PriceQuote, UnvaluedReason, quote_price};

/// Places for an average cost per share, the same as entered prices.
pub const AVERAGE_SCALE: u32 = 8;

/// One instrument's history up to `as_of`, added up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holding {
    pub instrument_id: Uuid,
    pub name: String,
    pub symbol: Option<String>,
    pub asset_class: AssetClass,
    pub quote_currency: String,
    /// Open quantity; zero for a sold position.
    pub quantity: Decimal,
    /// Cost of the open lots, without fees or taxes (ADR-LEDG-11).
    pub cost_basis: Decimal,
    /// Cost of every purchase and opening position, without fees.
    pub total_bought: Decimal,
    /// Realized profit or loss of every disposal (FIFO, before fees).
    pub realized: Decimal,
    /// Dividends received: amount − tax − fee.
    pub dividends: Decimal,
    pub fees: Decimal,
    /// May be negative when tax returns exceed taxes.
    pub taxes: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldingReturn {
    pub holding: Holding,
    /// `None` for a sold position.
    pub average_cost: Option<Decimal>,
    pub price: Option<PriceQuote>,
    /// quantity × price; zero for a sold position.
    pub market_value: Option<Decimal>,
    pub capital_gain: Option<Decimal>,
    pub capital_gain_percent: Option<Decimal>,
    pub total_profit: Option<Decimal>,
    pub total_profit_percent: Option<Decimal>,
    pub unvalued: Option<UnvaluedReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Returns {
    pub portfolio_id: Uuid,
    pub as_of: Date,
    /// Open positions first, then sold ones; each by name.
    pub holdings: Vec<HoldingReturn>,
    /// Some open position has no price or an out-of-range value.
    pub partial: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ReturnsError {
    #[error("validation failed")]
    Validation(Vec<FieldError>),
    #[error("portfolio not found")]
    NotFound,
    #[error("database error: {0}")]
    Database(#[from] DbErr),
}

/// `part / whole × 100`, rounded; `None` when `whole` is not positive.
fn percent(part: Decimal, whole: Decimal) -> Result<Option<Decimal>, UnvaluedReason> {
    if whole <= Decimal::ZERO {
        return Ok(None);
    }
    let scaled = exact_mul(part, Decimal::ONE_HUNDRED).ok_or(UnvaluedReason::OutOfRange)?;
    signed_div_round(scaled, whole, PERCENT_SCALE)
        .map(Some)
        .ok_or(UnvaluedReason::OutOfRange)
}

/// Everything except capital gain, which needs a price.
fn other_profit(h: &Holding) -> Option<Decimal> {
    let gross = exact_add(h.realized, h.dividends)?;
    exact_sub(exact_sub(gross, h.fees)?, h.taxes)
}

fn holding_return(holding: Holding, as_of: Date, price: Option<PricePoint>) -> HoldingReturn {
    let open = !holding.quantity.is_zero();
    let price = if open {
        price.map(|p| quote_price(as_of, p))
    } else {
        None
    };
    let average_cost = if open {
        div_round(holding.cost_basis, holding.quantity, AVERAGE_SCALE)
    } else {
        None
    };
    let mut result = HoldingReturn {
        holding,
        average_cost,
        price,
        market_value: None,
        capital_gain: None,
        capital_gain_percent: None,
        total_profit: None,
        total_profit_percent: None,
        unvalued: None,
    };
    let market_value = if !open {
        Ok(Decimal::ZERO)
    } else if let Some(price) = result.price {
        exact_mul(result.holding.quantity, price.price).ok_or(UnvaluedReason::OutOfRange)
    } else {
        Err(UnvaluedReason::MissingPrice)
    };
    let figures = market_value.and_then(|market_value| {
        let h = &result.holding;
        let gain = exact_sub(market_value, h.cost_basis).ok_or(UnvaluedReason::OutOfRange)?;
        let total = other_profit(h)
            .and_then(|other| exact_add(gain, other))
            .ok_or(UnvaluedReason::OutOfRange)?;
        Ok((
            market_value,
            gain,
            percent(gain, h.cost_basis)?,
            total,
            percent(total, h.total_bought)?,
        ))
    });
    match figures {
        Ok((market_value, gain, gain_percent, total, total_percent)) => {
            result.market_value = Some(market_value);
            result.capital_gain = Some(gain);
            result.capital_gain_percent = gain_percent;
            result.total_profit = Some(total);
            result.total_profit_percent = total_percent;
        }
        Err(reason) => result.unvalued = Some(reason),
    }
    result
}

/// Computes each holding's returns with its latest price on or before
/// `as_of` (`prices` by instrument).
pub fn compute(
    portfolio_id: Uuid,
    as_of: Date,
    holdings: Vec<Holding>,
    prices: &BTreeMap<Uuid, PricePoint>,
) -> Returns {
    let mut holdings: Vec<_> = holdings
        .into_iter()
        .map(|h| {
            let price = prices.get(&h.instrument_id).copied();
            holding_return(h, as_of, price)
        })
        .collect();
    holdings.sort_by(|a, b| {
        let (a, b) = (&a.holding, &b.holding);
        a.quantity
            .is_zero()
            .cmp(&b.quantity.is_zero())
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.instrument_id.cmp(&b.instrument_id))
    });
    Returns {
        portfolio_id,
        as_of,
        partial: holdings.iter().any(|h| h.unvalued.is_some()),
        holdings,
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use time::macros::date;

    use super::*;
    use crate::market_data::Source;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    fn holding(quantity: &str, cost: &str) -> Holding {
        Holding {
            instrument_id: Uuid::now_v7(),
            name: "AMZN".into(),
            symbol: Some("AMZN".into()),
            asset_class: AssetClass::Stock,
            quote_currency: "USD".into(),
            quantity: d(quantity),
            cost_basis: d(cost),
            total_bought: d("0"),
            realized: d("0"),
            dividends: d("0"),
            fees: d("0"),
            taxes: d("0"),
        }
    }

    fn price(value: &str) -> Option<PricePoint> {
        Some(PricePoint {
            price: d(value),
            date: date!(2026 - 10 - 30),
            source: Source::Manual,
        })
    }

    #[test]
    fn total_profit_adds_gain_realized_and_dividends_less_fees_and_taxes() {
        // Snowball's AMZN: capital gain 1.24, realized 582.57, fees 4.31.
        let mut h = holding("1.0655395", "254.86");
        h.total_bought = d("3590.66");
        h.realized = d("582.57");
        h.fees = d("4.31");
        let r = holding_return(h, date!(2026 - 11 - 01), price("240.35"));

        assert_eq!(r.market_value, Some(d("256.102418825")));
        assert_eq!(r.capital_gain, Some(d("1.242418825")));
        assert_eq!(r.capital_gain_percent, Some(d("0.4875")));
        assert_eq!(r.total_profit, Some(d("579.502418825")));
        assert_eq!(r.total_profit_percent, Some(d("16.1392")));
        assert_eq!(r.average_cost, Some(d("239.18400022")));
        assert!(!r.price.unwrap().stale);
        assert_eq!(r.unvalued, None);
    }

    #[test]
    fn dividends_count_and_tax_returns_lower_taxes() {
        let mut h = holding("10", "100");
        h.total_bought = d("100");
        h.dividends = d("8.5");
        h.taxes = d("-1");
        let r = holding_return(h, date!(2026 - 11 - 01), price("9"));
        // −10 + 8.5 − 0 − (−1)
        assert_eq!(r.total_profit, Some(d("-0.5")));
        assert_eq!(r.capital_gain_percent, Some(d("-10")));
        assert_eq!(r.total_profit_percent, Some(d("-0.5")));
    }

    #[test]
    fn a_sold_position_has_no_gain_but_keeps_its_profit() {
        let mut h = holding("0", "0");
        h.total_bought = d("500");
        h.realized = d("40");
        h.dividends = d("2");
        h.fees = d("1");
        let r = holding_return(h, date!(2026 - 11 - 01), price("99"));
        assert_eq!(r.price, None);
        assert_eq!(r.average_cost, None);
        assert_eq!(r.market_value, Some(d("0")));
        assert_eq!(r.capital_gain, Some(d("0")));
        assert_eq!(r.capital_gain_percent, None);
        assert_eq!(r.total_profit, Some(d("41")));
        assert_eq!(r.total_profit_percent, Some(d("8.2")));
    }

    #[test]
    fn an_open_position_without_a_price_is_never_valued_at_zero() {
        let mut h = holding("2", "10");
        h.realized = d("5");
        let id = h.instrument_id;
        let result = compute(
            Uuid::now_v7(),
            date!(2026 - 11 - 01),
            vec![h],
            &BTreeMap::new(),
        );
        let r = &result.holdings[0];
        assert!(result.partial);
        assert_eq!(r.holding.instrument_id, id);
        assert_eq!(r.unvalued, Some(UnvaluedReason::MissingPrice));
        assert_eq!(r.market_value, None);
        assert_eq!(r.total_profit, None);
        assert_eq!(r.average_cost, Some(d("5")));
    }

    #[test]
    fn open_positions_come_before_sold_ones_by_name() {
        let mut sold = holding("0", "0");
        sold.name = "Apple".into();
        let mut b = holding("1", "1");
        b.name = "Beta".into();
        let mut a = holding("1", "1");
        a.name = "Alpha".into();
        let prices: BTreeMap<_, _> = [a.instrument_id, b.instrument_id]
            .into_iter()
            .map(|id| (id, price("1").unwrap()))
            .collect();
        let result = compute(
            Uuid::now_v7(),
            date!(2026 - 11 - 01),
            vec![sold, b, a],
            &prices,
        );
        let names: Vec<_> = result
            .holdings
            .iter()
            .map(|h| h.holding.name.as_str())
            .collect();
        assert_eq!(names, ["Alpha", "Beta", "Apple"]);
        assert!(!result.partial);
    }
}
