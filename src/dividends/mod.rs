//! Dividends received, as in Snowball Analytics' "Dividends received"
//! charts (ADR-DIV-01).
//!
//! This module is pure: the service loads the payments and the FX rate in
//! force on each payment date, and the functions here add them up.
//!
//! - A payment is a `DIVIDEND` or an `INTEREST` transaction. Snowball counts
//!   income from cash-like custom assets with dividends, and Merton records
//!   that income as interest on a cash account.
//! - Each payment counts what was received: amount − tax − fee, exactly.
//! - It is converted to the base currency at the latest rate on or before
//!   its own date, rounded half-to-even to [`VALUE_SCALE`] places once
//!   ([`convert`]). Totals add those rounded values exactly, so the groups
//!   always sum to the total.
//! - A payment without a rate is never counted as zero: it is left out of
//!   every total, listed in `unconverted`, and the result is partial.

pub mod service;

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use sea_orm::DbErr;
use time::{Date, Month};
use uuid::Uuid;

use crate::auth::FieldError;
use crate::instruments::AssetClass;
use crate::ledger::decimal::exact_add;
use crate::ledger::transaction::TransactionKind;
use crate::valuation::{AllocationClass, FxPoint, UnvaluedReason, VALUE_SCALE, convert};

/// What a payment came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A dividend from a holding.
    Instrument {
        id: Uuid,
        name: String,
        symbol: Option<String>,
        asset_class: AssetClass,
    },
    /// Interest paid on a cash account.
    CashAccount { id: Uuid, name: String },
}

impl Source {
    fn id(&self) -> Uuid {
        match self {
            Self::Instrument { id, .. } | Self::CashAccount { id, .. } => *id,
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Instrument { name, .. } | Self::CashAccount { name, .. } => name,
        }
    }

    fn class(&self) -> AllocationClass {
        match self {
            Self::Instrument { asset_class, .. } => (*asset_class).into(),
            Self::CashAccount { .. } => AllocationClass::Cash,
        }
    }
}

/// One dividend or interest payment, in its own currency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payment {
    pub transaction_id: Uuid,
    pub kind: TransactionKind,
    pub trade_date: Date,
    pub source: Source,
    pub currency: String,
    /// Amount − tax − fee, exact.
    pub received: Decimal,
    /// The rate in force on `trade_date`; `None` for the base currency or
    /// when no rate is stored.
    pub fx: Option<FxPoint>,
}

/// Received amounts of one calendar month, in the base currency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonthTotal {
    pub year: i32,
    pub month: Month,
    pub received: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTotal {
    pub source: Source,
    pub currency: String,
    pub received: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassTotal {
    pub class: AllocationClass,
    pub received: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrencyTotal {
    /// The currency the payments were made in.
    pub currency: String,
    /// Their total in the base currency.
    pub received: Decimal,
}

/// A payment left out of the totals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unconverted {
    pub payment: Payment,
    pub reason: UnvaluedReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dividends {
    pub portfolio_id: Uuid,
    pub base_currency: String,
    pub from: Option<Date>,
    pub to: Option<Date>,
    pub total: Decimal,
    /// Months with a converted payment, oldest first.
    pub months: Vec<MonthTotal>,
    /// Largest first.
    pub sources: Vec<SourceTotal>,
    /// In asset-class order, `CASH` for interest.
    pub classes: Vec<ClassTotal>,
    /// Largest first.
    pub currencies: Vec<CurrencyTotal>,
    /// In ledger order.
    pub unconverted: Vec<Unconverted>,
    pub partial: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum DividendsError {
    #[error("validation failed")]
    Validation(Vec<FieldError>),
    #[error("portfolio not found")]
    NotFound,
    /// A total is too large to represent exactly.
    #[error("the dividends total is out of range")]
    OutOfRange,
    #[error("database error: {0}")]
    Database(#[from] DbErr),
}

fn zero() -> Decimal {
    Decimal::new(0, VALUE_SCALE)
}

fn add_to(total: &mut Decimal, value: Decimal) -> Result<(), DividendsError> {
    *total = exact_add(*total, value).ok_or(DividendsError::OutOfRange)?;
    Ok(())
}

/// Adds up `payments` (in ledger order) in `base_currency`.
pub fn summarize(
    portfolio_id: Uuid,
    base_currency: String,
    from: Option<Date>,
    to: Option<Date>,
    payments: Vec<Payment>,
) -> Result<Dividends, DividendsError> {
    let mut total = zero();
    let mut months: BTreeMap<(i32, u8), Decimal> = BTreeMap::new();
    let mut sources: BTreeMap<Uuid, SourceTotal> = BTreeMap::new();
    let mut classes: BTreeMap<AllocationClass, Decimal> = BTreeMap::new();
    let mut currencies: BTreeMap<String, Decimal> = BTreeMap::new();
    let mut unconverted = Vec::new();

    for payment in payments {
        let value = match convert(
            payment.received,
            &payment.currency,
            &base_currency,
            payment.fx,
        ) {
            Ok(value) => value,
            Err(reason) => {
                unconverted.push(Unconverted { payment, reason });
                continue;
            }
        };
        add_to(&mut total, value)?;
        let date = payment.trade_date;
        add_to(
            months
                .entry((date.year(), u8::from(date.month())))
                .or_insert_with(zero),
            value,
        )?;
        add_to(
            classes.entry(payment.source.class()).or_insert_with(zero),
            value,
        )?;
        add_to(
            currencies
                .entry(payment.currency.clone())
                .or_insert_with(zero),
            value,
        )?;
        let entry = sources
            .entry(payment.source.id())
            .or_insert_with(|| SourceTotal {
                source: payment.source,
                currency: payment.currency,
                received: zero(),
            });
        add_to(&mut entry.received, value)?;
    }

    let months = months
        .into_iter()
        .filter_map(|((year, month), received)| {
            Some(MonthTotal {
                year,
                month: Month::try_from(month).ok()?,
                received,
            })
        })
        .collect();
    let mut sources: Vec<_> = sources.into_values().collect();
    sources.sort_by(|a, b| {
        b.received
            .cmp(&a.received)
            .then_with(|| a.source.name().cmp(b.source.name()))
            .then_with(|| a.source.id().cmp(&b.source.id()))
    });
    let mut currencies: Vec<_> = currencies
        .into_iter()
        .map(|(currency, received)| CurrencyTotal { currency, received })
        .collect();
    currencies.sort_by(|a, b| {
        b.received
            .cmp(&a.received)
            .then_with(|| a.currency.cmp(&b.currency))
    });
    Ok(Dividends {
        portfolio_id,
        base_currency,
        from,
        to,
        total,
        months,
        sources,
        classes: classes
            .into_iter()
            .map(|(class, received)| ClassTotal { class, received })
            .collect(),
        currencies,
        partial: !unconverted.is_empty(),
        unconverted,
    })
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use time::macros::date;

    use super::*;
    use crate::market_data::Source as RateSource;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    fn stock(name: &str) -> Source {
        Source::Instrument {
            id: Uuid::now_v7(),
            name: name.to_owned(),
            symbol: Some(name.to_owned()),
            asset_class: AssetClass::Stock,
        }
    }

    fn payment(source: &Source, date: Date, currency: &str, received: &str) -> Payment {
        Payment {
            transaction_id: Uuid::now_v7(),
            kind: TransactionKind::Dividend,
            trade_date: date,
            source: source.clone(),
            currency: currency.to_owned(),
            received: d(received),
            fx: None,
        }
    }

    fn rate(rate: &str, inverse: bool) -> Option<FxPoint> {
        Some(FxPoint {
            rate: d(rate),
            date: date!(2026 - 01 - 01),
            source: RateSource::Manual,
            inverse,
        })
    }

    #[test]
    fn groups_add_up_to_the_total_by_month_source_class_and_currency() {
        let pep = stock("PEP");
        let ko = stock("KO");
        let cash = Source::CashAccount {
            id: Uuid::now_v7(),
            name: "Dime! FCD".to_owned(),
        };
        let mut interest = payment(&cash, date!(2026 - 02 - 28), "USD", "75.20547945");
        interest.kind = TransactionKind::Interest;
        let payments = vec![
            payment(&pep, date!(2026 - 01 - 05), "USD", "10.5"),
            payment(&ko, date!(2026 - 01 - 20), "USD", "2.25"),
            payment(&pep, date!(2026 - 02 - 05), "USD", "10.5"),
            interest,
        ];
        let result = summarize(Uuid::now_v7(), "USD".into(), None, None, payments).unwrap();

        assert_eq!(result.total, d("98.45547945"));
        assert!(!result.partial);
        let months: Vec<_> = result
            .months
            .iter()
            .map(|m| (m.year, m.month, m.received))
            .collect();
        assert_eq!(
            months,
            [
                (2026, Month::January, d("12.75")),
                (2026, Month::February, d("85.70547945")),
            ]
        );
        let sources: Vec<_> = result
            .sources
            .iter()
            .map(|s| (s.source.name().to_owned(), s.received))
            .collect();
        assert_eq!(
            sources,
            [
                ("Dime! FCD".to_owned(), d("75.20547945")),
                ("PEP".to_owned(), d("21")),
                ("KO".to_owned(), d("2.25")),
            ]
        );
        let classes: Vec<_> = result
            .classes
            .iter()
            .map(|c| (c.class, c.received))
            .collect();
        assert_eq!(
            classes,
            [
                (AllocationClass::Stock, d("23.25")),
                (AllocationClass::Cash, d("75.20547945")),
            ]
        );
        assert_eq!(result.currencies.len(), 1);
        assert_eq!(result.currencies[0].received, result.total);
    }

    #[test]
    fn converts_each_payment_at_its_own_rate_and_rounds_once() {
        let ptt = stock("PTT");
        let mut direct = payment(&ptt, date!(2026 - 03 - 01), "THB", "100");
        // 1 THB = 0.0299401197 USD: 2.99401197, already 8 places.
        direct.fx = rate("0.0299401197", false);
        let mut inverse = payment(&ptt, date!(2026 - 04 - 01), "THB", "100");
        // 1 USD = 33.4 THB: 100 / 33.4 = 2.99401197604… → 2.99401198.
        inverse.fx = rate("33.4", true);
        let result = summarize(
            Uuid::now_v7(),
            "USD".into(),
            None,
            None,
            vec![direct, inverse],
        )
        .unwrap();

        assert_eq!(result.total, d("5.98802395"));
        assert_eq!(result.months[1].received, d("2.99401198"));
        assert_eq!(result.currencies[0].currency, "THB");
    }

    #[test]
    fn a_payment_without_a_rate_is_listed_and_never_counted_as_zero() {
        let ptt = stock("PTT");
        let pep = stock("PEP");
        let payments = vec![
            payment(&ptt, date!(2026 - 03 - 01), "THB", "100"),
            payment(&pep, date!(2026 - 03 - 02), "USD", "1"),
        ];
        let result = summarize(Uuid::now_v7(), "USD".into(), None, None, payments).unwrap();

        assert!(result.partial);
        assert_eq!(result.total, d("1"));
        assert_eq!(result.unconverted.len(), 1);
        assert_eq!(result.unconverted[0].reason, UnvaluedReason::MissingFxRate);
        assert_eq!(result.unconverted[0].payment.received, d("100"));
        assert_eq!(result.sources.len(), 1);
    }

    #[test]
    fn no_payments_gives_a_zero_total_and_empty_groups() {
        let result = summarize(Uuid::now_v7(), "USD".into(), None, None, Vec::new()).unwrap();
        assert!(result.total.is_zero());
        assert!(result.months.is_empty() && result.sources.is_empty());
        assert!(!result.partial);
    }
}
