//! Loads dividend and interest payments with the FX rate in force on each
//! payment date, and adds them up.
//!
//! Everything is read in one read-only transaction, so the result sees a
//! consistent snapshot.

use std::collections::{BTreeMap, BTreeSet};

use sea_orm::{DatabaseConnection, DbErr, TransactionTrait};
use time::Date;
use uuid::Uuid;

use super::{Dividends, DividendsError, Payment, Source, summarize};
use crate::auth::{FieldError, UserId};
use crate::cash_accounts::repository as cash_accounts;
use crate::infrastructure::entities::{fx_rate, transaction};
use crate::instruments::AssetClass;
use crate::instruments::repository as instruments;
use crate::ledger::decimal::{Amount, Rate, exact_sub};
use crate::ledger::repository as ledger;
use crate::ledger::transaction::{TransactionKind, parse_date};
use crate::market_data::{Source as RateSource, repository as market_data};
use crate::portfolios::repository as portfolios;
use crate::valuation::{StoredRate, choose_rate};

/// Dividends and interest received between `from` and `to` (both
/// inclusive, both optional), in the portfolio's base currency.
pub async fn dividends(
    db: &DatabaseConnection,
    owner: UserId,
    portfolio_id: Uuid,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Dividends, DividendsError> {
    let (from, to) = parse_range(from, to)?;
    let txn = db.begin().await?;
    let portfolio = portfolios::find(&txn, owner, portfolio_id)
        .await?
        .ok_or(DividendsError::NotFound)?;
    let base = portfolio.base_currency;
    let models = ledger::income_between(&txn, owner, portfolio_id, from, to).await?;

    let instrument_ids: Vec<Uuid> = models
        .iter()
        .filter_map(|m| m.instrument_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let instruments: BTreeMap<Uuid, _> = instruments::find_many(&txn, owner, instrument_ids)
        .await?
        .into_iter()
        .map(|m| (m.id, m))
        .collect();
    let accounts: BTreeMap<Uuid, _> = cash_accounts::all_in_portfolio(&txn, owner, portfolio_id)
        .await?
        .into_iter()
        .map(|m| (m.id, m))
        .collect();

    let mut payments = Vec::with_capacity(models.len());
    for model in &models {
        let kind = TransactionKind::from_db(&model.kind)?;
        let (source, currency) = match kind {
            TransactionKind::Dividend => {
                let id = model
                    .instrument_id
                    .ok_or_else(|| corrupt(model, "has no instrument"))?;
                let instrument = instruments
                    .get(&id)
                    .ok_or_else(|| corrupt(model, "names an instrument that is not owned"))?;
                (
                    Source::Instrument {
                        id,
                        name: instrument.name.clone(),
                        symbol: instrument.symbol.clone(),
                        asset_class: AssetClass::from_db(&instrument.asset_class)?,
                    },
                    instrument.quote_currency.clone(),
                )
            }
            _ => {
                let id = model
                    .cash_account_id
                    .ok_or_else(|| corrupt(model, "has no cash account"))?;
                let account = accounts
                    .get(&id)
                    .ok_or_else(|| corrupt(model, "names a cash account outside the portfolio"))?;
                (
                    Source::CashAccount {
                        id,
                        name: account.name.clone(),
                    },
                    account.currency.clone(),
                )
            }
        };
        payments.push(Payment {
            transaction_id: model.id,
            kind,
            trade_date: model.trade_date,
            source,
            currency,
            received: received(model)?,
            fx: None,
        });
    }

    // Every rate up to the last payment, for each foreign currency.
    let last = payments.iter().map(|p| p.trade_date).max();
    let mut rates: BTreeMap<String, (Vec<StoredRate>, Vec<StoredRate>)> = BTreeMap::new();
    if let Some(last) = last {
        let foreign: BTreeSet<&str> = payments
            .iter()
            .map(|p| p.currency.as_str())
            .filter(|c| *c != base)
            .collect();
        for currency in foreign {
            let direct = market_data::fx_rates_until(&txn, owner, currency, &base, last).await?;
            let inverse = market_data::fx_rates_until(&txn, owner, &base, currency, last).await?;
            rates.insert(currency.to_owned(), (stored(direct)?, stored(inverse)?));
        }
    }
    txn.commit().await?;

    for payment in &mut payments {
        if let Some((direct, inverse)) = rates.get(&payment.currency) {
            payment.fx = choose_rate(
                latest_on(direct, payment.trade_date),
                latest_on(inverse, payment.trade_date),
            );
        }
    }
    summarize(portfolio_id, base, from, to, payments)
}

fn parse_range(
    from: Option<&str>,
    to: Option<&str>,
) -> Result<(Option<Date>, Option<Date>), DividendsError> {
    let mut errors = Vec::new();
    let mut date = |field: &'static str, input: Option<&str>| {
        input.and_then(|s| match parse_date(s) {
            Ok(d) => Some(d),
            Err(message) => {
                errors.push(FieldError { field, message });
                None
            }
        })
    };
    let from_date = date("from", from);
    let to_date = date("to", to);
    if let (Some(f), Some(t)) = (from_date, to_date)
        && t < f
    {
        errors.push(FieldError {
            field: "to",
            message: "must be on or after from".to_owned(),
        });
    }
    if errors.is_empty() {
        Ok((from_date, to_date))
    } else {
        Err(DividendsError::Validation(errors))
    }
}

fn corrupt(model: &transaction::Model, problem: &str) -> DividendsError {
    DividendsError::Database(DbErr::Type(format!("transaction {} {problem}", model.id)))
}

/// Amount − tax − fee, exact.
fn received(model: &transaction::Model) -> Result<rust_decimal::Decimal, DividendsError> {
    let read = |text: Option<&String>| -> Result<rust_decimal::Decimal, DividendsError> {
        text.map(|t| Amount::from_stored(t).map(Amount::value))
            .transpose()
            .map(Option::unwrap_or_default)
            .map_err(|err| corrupt(model, &err))
    };
    let amount = model
        .amount
        .as_ref()
        .ok_or_else(|| corrupt(model, "has no amount"))?;
    let amount = read(Some(amount))?;
    let tax = read(model.tax.as_ref())?;
    let fee = read(model.fee.as_ref())?;
    exact_sub(amount, tax)
        .and_then(|v| exact_sub(v, fee))
        .ok_or(DividendsError::OutOfRange)
}

fn stored(models: Vec<fx_rate::Model>) -> Result<Vec<StoredRate>, DividendsError> {
    models
        .into_iter()
        .map(|model| {
            let rate = Rate::from_stored(&model.rate).map_err(|err| {
                DividendsError::Database(DbErr::Type(format!("fx rate {}: {err}", model.id)))
            })?;
            Ok(StoredRate {
                rate: rate.value(),
                date: model.rate_date,
                source: RateSource::from_db(&model.source)?,
            })
        })
        .collect()
}

/// The latest of `rates` (oldest first) on or before `date`.
fn latest_on(rates: &[StoredRate], date: Date) -> Option<StoredRate> {
    let after = rates.partition_point(|r| r.date <= date);
    rates.get(after.checked_sub(1)?).copied()
}
