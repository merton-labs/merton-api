//! Loads holdings and market data at `as_of` and values them.
//!
//! Everything is read in one read-only transaction, so a valuation sees a
//! consistent snapshot. Only data dated on or before `as_of` is read: no
//! later transaction, price or rate can leak in.

use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use sea_orm::{DatabaseConnection, DatabaseTransaction, DbErr, TransactionTrait};
use time::Date;
use uuid::Uuid;

use super::{
    Allocation, CashBalance, Holdings, MarketData, Position, PricePoint, StoredRate, Valuation,
    ValuationError, allocate, choose_rate, value,
};
use crate::auth::UserId;
use crate::cash_accounts::repository as cash_accounts;
use crate::infrastructure::entities::fx_rate;
use crate::instruments::AssetClass;
use crate::instruments::repository as instruments;
use crate::ledger::decimal::{Price, Rate, SignedAmount, exact_add};
use crate::ledger::service::to_event;
use crate::ledger::transaction::parse_date;
use crate::ledger::{fifo, repository as ledger};
use crate::market_data::{Source, repository as market_data};
use crate::portfolios::repository as portfolios;
use crate::validation::check;

/// Open positions and cash balances at `as_of`, without prices.
pub async fn holdings(
    db: &DatabaseConnection,
    owner: UserId,
    portfolio_id: Uuid,
    as_of: Option<&str>,
) -> Result<Holdings, ValuationError> {
    let as_of = parse_as_of(as_of)?;
    let txn = db.begin().await?;
    let holdings = load_holdings(&txn, owner, portfolio_id, as_of).await?;
    txn.commit().await?;
    Ok(holdings)
}

/// Values the portfolio at `as_of` in its base currency.
pub async fn valuation(
    db: &DatabaseConnection,
    owner: UserId,
    portfolio_id: Uuid,
    as_of: Option<&str>,
) -> Result<Valuation, ValuationError> {
    let as_of = parse_as_of(as_of)?;
    let txn = db.begin().await?;
    let holdings = load_holdings(&txn, owner, portfolio_id, as_of).await?;
    let market = load_market_data(&txn, owner, &holdings).await?;
    txn.commit().await?;
    value(holdings, &market)
}

/// Signed allocation by asset class at `as_of`.
pub async fn allocation(
    db: &DatabaseConnection,
    owner: UserId,
    portfolio_id: Uuid,
    as_of: Option<&str>,
) -> Result<Allocation, ValuationError> {
    let valuation = valuation(db, owner, portfolio_id, as_of).await?;
    allocate(&valuation)
}

/// `as_of` is required: valuations are always for an explicit date
/// (ADR-VAL-01).
fn parse_as_of(as_of: Option<&str>) -> Result<Date, ValuationError> {
    let mut errors = Vec::new();
    let parsed = check(
        &mut errors,
        "as_of",
        as_of
            .ok_or_else(|| "is required".to_owned())
            .and_then(parse_date),
    );
    parsed.ok_or(ValuationError::Validation(errors))
}

fn corrupt(err: String) -> ValuationError {
    ValuationError::Database(DbErr::Type(err))
}

async fn load_holdings(
    txn: &DatabaseTransaction,
    owner: UserId,
    portfolio_id: Uuid,
    as_of: Date,
) -> Result<Holdings, ValuationError> {
    let portfolio = portfolios::find(txn, owner, portfolio_id)
        .await?
        .ok_or(ValuationError::NotFound)?;

    // Replay each instrument's history up to `as_of`. History was validated
    // when it was written, and any prefix of a valid history is valid.
    let mut events: BTreeMap<Uuid, Vec<fifo::Event>> = BTreeMap::new();
    for movement in ledger::positions_until(txn, owner, portfolio_id, as_of).await? {
        events
            .entry(movement.instrument_id)
            .or_default()
            .push(to_event(&movement)?);
    }
    let mut open = BTreeMap::new();
    for (instrument_id, events) in events {
        let replay = fifo::replay(events).map_err(|err| {
            corrupt(format!(
                "instrument {instrument_id} history does not replay: {err}"
            ))
        })?;
        if !replay.quantity().is_zero() {
            open.insert(instrument_id, (replay.quantity(), replay.cost()));
        }
    }

    let models = instruments::find_many(txn, owner, open.keys().copied().collect()).await?;
    let mut positions = Vec::with_capacity(open.len());
    for model in models {
        let Some((quantity, cost_basis)) = open.remove(&model.id) else {
            continue;
        };
        positions.push(Position {
            instrument_id: model.id,
            asset_class: AssetClass::from_db(&model.asset_class)?,
            name: model.name,
            symbol: model.symbol,
            quote_currency: model.quote_currency,
            quantity,
            cost_basis,
        });
    }
    if let Some(missing) = open.keys().next() {
        return Err(corrupt(format!("held instrument {missing} is not owned")));
    }
    positions.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then(a.instrument_id.cmp(&b.instrument_id))
    });

    let mut balances: BTreeMap<Uuid, Decimal> = BTreeMap::new();
    for movement in ledger::cash_until(txn, owner, portfolio_id, as_of).await? {
        let delta = SignedAmount::from_stored(&movement.amount_delta)
            .map_err(|err| corrupt(format!("cash movement {}: {err}", movement.id)))?;
        let balance = balances.entry(movement.cash_account_id).or_default();
        *balance = exact_add(*balance, delta.value()).ok_or(ValuationError::OutOfRange)?;
    }
    let cash = cash_accounts::all_in_portfolio(txn, owner, portfolio_id)
        .await?
        .into_iter()
        .map(|account| CashBalance {
            balance: balances.get(&account.id).copied().unwrap_or_default(),
            cash_account_id: account.id,
            name: account.name,
            currency: account.currency,
        })
        .collect();

    Ok(Holdings {
        portfolio_id,
        as_of,
        base_currency: portfolio.base_currency,
        positions,
        cash,
    })
}

async fn load_market_data(
    txn: &DatabaseTransaction,
    owner: UserId,
    holdings: &Holdings,
) -> Result<MarketData, ValuationError> {
    let as_of = holdings.as_of;
    let base = holdings.base_currency.as_str();
    let mut market = MarketData::default();

    for position in &holdings.positions {
        let id = position.instrument_id;
        if let Some(model) = market_data::latest_price(txn, owner, id, as_of).await? {
            let price = Price::from_stored(&model.price)
                .map_err(|err| corrupt(format!("price {}: {err}", model.id)))?;
            market.prices.insert(
                id,
                PricePoint {
                    price: price.value(),
                    date: model.price_date,
                    source: Source::from_db(&model.source)?,
                },
            );
        }
    }

    let currencies: BTreeSet<&str> = holdings
        .positions
        .iter()
        .map(|p| p.quote_currency.as_str())
        .chain(holdings.cash.iter().map(|c| c.currency.as_str()))
        .filter(|c| *c != base)
        .collect();
    for currency in currencies {
        let direct = market_data::latest_fx_rate(txn, owner, currency, base, as_of).await?;
        let inverse = market_data::latest_fx_rate(txn, owner, base, currency, as_of).await?;
        if let Some(point) = choose_rate(stored(direct)?, stored(inverse)?) {
            market.rates.insert(currency.to_owned(), point);
        }
    }
    Ok(market)
}

fn stored(model: Option<fx_rate::Model>) -> Result<Option<StoredRate>, ValuationError> {
    model
        .map(|model| {
            let rate = Rate::from_stored(&model.rate)
                .map_err(|err| corrupt(format!("fx rate {}: {err}", model.id)))?;
            Ok(StoredRate {
                rate: rate.value(),
                date: model.rate_date,
                source: Source::from_db(&model.source)?,
            })
        })
        .transpose()
}
