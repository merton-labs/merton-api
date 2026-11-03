//! Replays every instrument of a portfolio up to `as_of`, adds up its
//! transactions, and computes its returns.
//!
//! Everything is read in one read-only transaction, so the result sees a
//! consistent snapshot. Only data dated on or before `as_of` is read.

use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use sea_orm::{DatabaseConnection, DbErr};
use time::Date;
use uuid::Uuid;

use super::{Holding, Returns, ReturnsError, compute};
use crate::auth::UserId;
use crate::infrastructure::database::begin_read;
use crate::infrastructure::entities::transaction;
use crate::instruments::AssetClass;
use crate::instruments::repository as instruments;
use crate::ledger::decimal::{Amount, Price, exact_add, exact_sub};
use crate::ledger::fifo::{self, Event};
use crate::ledger::repository as ledger;
use crate::ledger::service::to_event;
use crate::ledger::transaction::{TransactionKind, parse_date};
use crate::market_data::{Source, repository as market_data};
use crate::portfolios::repository as portfolios;
use crate::validation::check;
use crate::valuation::PricePoint;

/// Totals of one instrument's transactions.
#[derive(Default)]
struct Sums {
    dividends: Decimal,
    fees: Decimal,
    taxes: Decimal,
}

/// Each instrument's returns at `as_of`, in its quote currency.
pub async fn returns(
    db: &DatabaseConnection,
    owner: UserId,
    portfolio_id: Uuid,
    as_of: Option<&str>,
) -> Result<Returns, ReturnsError> {
    let as_of = parse_as_of(as_of)?;
    let txn = begin_read(db).await?;
    portfolios::find(&txn, owner, portfolio_id)
        .await?
        .ok_or(ReturnsError::NotFound)?;

    let mut events: BTreeMap<Uuid, Vec<Event>> = BTreeMap::new();
    for movement in ledger::positions_until(&txn, owner, portfolio_id, as_of).await? {
        events
            .entry(movement.instrument_id)
            .or_default()
            .push(to_event(&movement)?);
    }

    let mut sums: BTreeMap<Uuid, Sums> = BTreeMap::new();
    for model in ledger::with_instrument_until(&txn, owner, portfolio_id, as_of).await? {
        let Some(instrument_id) = model.instrument_id else {
            continue;
        };
        add_transaction(sums.entry(instrument_id).or_default(), &model)?;
    }

    let ids: BTreeSet<Uuid> = events.keys().chain(sums.keys()).copied().collect();
    let models = instruments::find_many(&txn, owner, ids.iter().copied().collect()).await?;
    if models.len() != ids.len() {
        return Err(corrupt(
            "a transaction names an instrument that is not owned",
        ));
    }

    let mut holdings = Vec::with_capacity(models.len());
    let mut prices = BTreeMap::new();
    for model in models {
        let history = events.remove(&model.id).unwrap_or_default();
        let total_bought = history
            .iter()
            .filter_map(|e| match e {
                Event::Acquire { cost, .. } => Some(cost.value()),
                _ => None,
            })
            .try_fold(Decimal::ZERO, exact_add)
            .ok_or_else(|| corrupt("purchases out of range"))?;
        let replay = fifo::replay(history)
            .map_err(|err| corrupt(&format!("instrument {} does not replay: {err}", model.id)))?;
        let realized = replay
            .disposals
            .iter()
            .map(|d| d.realized)
            .try_fold(Decimal::ZERO, exact_add)
            .ok_or_else(|| corrupt("realized P&L out of range"))?;
        let sums = sums.remove(&model.id).unwrap_or_default();
        if !replay.quantity().is_zero()
            && let Some(price) = market_data::latest_price(&txn, owner, model.id, as_of).await?
        {
            let value = Price::from_stored(&price.price)
                .map_err(|err| corrupt(&format!("price {}: {err}", price.id)))?;
            prices.insert(
                model.id,
                PricePoint {
                    price: value.value(),
                    date: price.price_date,
                    source: Source::from_db(&price.source)?,
                },
            );
        }
        holdings.push(Holding {
            instrument_id: model.id,
            asset_class: AssetClass::from_db(&model.asset_class)?,
            name: model.name,
            symbol: model.symbol,
            quote_currency: model.quote_currency,
            quantity: replay.quantity(),
            cost_basis: replay.cost(),
            total_bought,
            realized,
            dividends: sums.dividends,
            fees: sums.fees,
            taxes: sums.taxes,
        });
    }
    txn.commit().await?;
    Ok(compute(portfolio_id, as_of, holdings, &prices))
}

/// `as_of` is required, as for valuation (ADR-VAL-01).
fn parse_as_of(as_of: Option<&str>) -> Result<Date, ReturnsError> {
    let mut errors = Vec::new();
    let parsed = check(
        &mut errors,
        "as_of",
        as_of
            .ok_or_else(|| "is required".to_owned())
            .and_then(parse_date),
    );
    parsed.ok_or(ReturnsError::Validation(errors))
}

fn corrupt(problem: &str) -> ReturnsError {
    ReturnsError::Database(DbErr::Type(problem.to_owned()))
}

/// Adds one transaction of an instrument to its totals.
fn add_transaction(sums: &mut Sums, model: &transaction::Model) -> Result<(), ReturnsError> {
    let read = |text: &Option<String>| -> Result<Decimal, ReturnsError> {
        text.as_deref()
            .map(|t| Amount::from_stored(t).map(Amount::value))
            .transpose()
            .map(Option::unwrap_or_default)
            .map_err(|err| corrupt(&format!("transaction {}: {err}", model.id)))
    };
    let (amount, fee, tax) = (read(&model.amount)?, read(&model.fee)?, read(&model.tax)?);
    let out_of_range = || corrupt(&format!("transaction {} totals out of range", model.id));
    let add = |total: &mut Decimal, value: Decimal| -> Result<(), ReturnsError> {
        *total = exact_add(*total, value).ok_or_else(out_of_range)?;
        Ok(())
    };
    match TransactionKind::from_db(&model.kind)? {
        TransactionKind::Dividend => {
            let received = exact_sub(amount, tax)
                .and_then(|v| exact_sub(v, fee))
                .ok_or_else(out_of_range)?;
            add(&mut sums.dividends, received)?;
        }
        TransactionKind::Fee | TransactionKind::OtherExpense => add(&mut sums.fees, amount)?,
        TransactionKind::Tax => add(&mut sums.taxes, amount)?,
        TransactionKind::TaxReturn => add(&mut sums.taxes, -amount)?,
        // Buys and sells: the fee and tax on the trade.
        _ => {
            add(&mut sums.fees, fee)?;
            add(&mut sums.taxes, tax)?;
        }
    }
    Ok(())
}
