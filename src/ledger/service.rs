//! Ledger use cases: record, replace, delete and read transactions.
//!
//! Every write runs in one database transaction (ADR-LEDG-03, ADR-DB-09):
//!
//! 1. lock the owned, `ACTIVE` portfolio with a guarded write;
//! 2. check that the referenced instrument belongs to the caller and the
//!    cash accounts to this portfolio, with matching currencies;
//! 3. write the transaction and rewrite its movements;
//! 4. replay FIFO for every affected instrument and reject the change if a
//!    holding would go negative anywhere in history (ADR-LEDG-04/09);
//! 5. commit. Any failure rolls everything back.

use std::collections::BTreeSet;

use sea_orm::{DatabaseConnection, DatabaseTransaction, DbErr, TransactionTrait};
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use super::decimal::{Amount, Quantity, Ratio, SignedAmount};
use super::fifo::{self, Event, FifoError};
use super::repository;
use super::transaction::{
    Details, MovementKind, RawTransaction, TransactionKind, ValidTransaction, validate,
};
use crate::auth::{FieldError, UserId};
use crate::cash_accounts::repository as cash_accounts;
use crate::infrastructure::database::is_foreign_key_violation;
use crate::infrastructure::entities::{cash_movement, position_movement, transaction};
use crate::instruments::repository as instruments;
use crate::pagination::{Page, Paged};
use crate::portfolios::repository as portfolios;

/// A recorded transaction. Decimal fields are canonical decimal strings, at
/// the scale they were entered with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerEntry {
    pub id: Uuid,
    pub portfolio_id: Uuid,
    pub kind: TransactionKind,
    pub trade_date: Date,
    pub sort_order: i32,
    pub instrument_id: Option<Uuid>,
    pub cash_account_id: Option<Uuid>,
    pub destination_cash_account_id: Option<Uuid>,
    pub quantity: Option<String>,
    pub ratio: Option<String>,
    pub price: Option<String>,
    pub fee: Option<String>,
    pub tax: Option<String>,
    pub amount: Option<String>,
    pub destination_amount: Option<String>,
    pub cash_in_lieu_quantity: Option<String>,
    pub note: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("validation failed")]
    Validation(Vec<FieldError>),
    /// The portfolio or the transaction does not exist for this user.
    #[error("transaction not found")]
    NotFound,
    #[error("portfolio is archived")]
    PortfolioArchived,
    #[error("holdings of instrument {instrument_id} would go negative on {trade_date}")]
    InsufficientHoldings {
        instrument_id: Uuid,
        trade_date: Date,
    },
    /// A split or stock dividend on a date with no holding to adjust.
    #[error("no holding of instrument {instrument_id} to adjust on {trade_date}")]
    NoHoldings {
        instrument_id: Uuid,
        trade_date: Date,
    },
    #[error("database error: {0}")]
    Database(#[from] DbErr),
}

pub async fn create(
    db: &DatabaseConnection,
    owner: UserId,
    portfolio_id: Uuid,
    kind: TransactionKind,
    raw: &RawTransaction<'_>,
) -> Result<LedgerEntry, LedgerError> {
    let valid = validate(kind, raw).map_err(LedgerError::Validation)?;
    let now = OffsetDateTime::now_utc();
    let model = to_model(Uuid::now_v7(), portfolio_id, &valid, now, now);

    write(db, owner, portfolio_id, |txn| async move {
        check_references(&txn, owner, portfolio_id, &valid).await?;
        repository::insert(&txn, model.clone()).await?;
        post_movements(&txn, &model, &valid).await?;
        replay(
            &txn,
            portfolio_id,
            valid.instrument_id().into_iter().collect(),
        )
        .await?;
        Ok((txn, model))
    })
    .await
}

/// Replaces a transaction entirely (PUT semantics), including its kind.
pub async fn replace(
    db: &DatabaseConnection,
    owner: UserId,
    portfolio_id: Uuid,
    id: Uuid,
    kind: TransactionKind,
    raw: &RawTransaction<'_>,
) -> Result<LedgerEntry, LedgerError> {
    let valid = validate(kind, raw).map_err(LedgerError::Validation)?;

    write(db, owner, portfolio_id, |txn| async move {
        let existing = repository::find(&txn, owner, portfolio_id, id)
            .await?
            .ok_or(LedgerError::NotFound)?;
        check_references(&txn, owner, portfolio_id, &valid).await?;
        let model = to_model(
            id,
            portfolio_id,
            &valid,
            existing.created_at,
            OffsetDateTime::now_utc(),
        );
        repository::delete_movements(&txn, id).await?;
        repository::replace(&txn, model.clone()).await?;
        post_movements(&txn, &model, &valid).await?;
        let affected = existing
            .instrument_id
            .into_iter()
            .chain(valid.instrument_id())
            .collect();
        replay(&txn, portfolio_id, affected).await?;
        Ok((txn, model))
    })
    .await
}

pub async fn delete(
    db: &DatabaseConnection,
    owner: UserId,
    portfolio_id: Uuid,
    id: Uuid,
) -> Result<(), LedgerError> {
    write(db, owner, portfolio_id, |txn| async move {
        let existing = repository::find(&txn, owner, portfolio_id, id)
            .await?
            .ok_or(LedgerError::NotFound)?;
        repository::delete_movements(&txn, id).await?;
        repository::delete(&txn, id).await?;
        replay(
            &txn,
            portfolio_id,
            existing.instrument_id.into_iter().collect(),
        )
        .await?;
        Ok((txn, existing))
    })
    .await
    .map(|_| ())
}

pub async fn get(
    db: &DatabaseConnection,
    owner: UserId,
    portfolio_id: Uuid,
    id: Uuid,
) -> Result<LedgerEntry, LedgerError> {
    let model = repository::find(db, owner, portfolio_id, id)
        .await?
        .ok_or(LedgerError::NotFound)?;
    to_entry(model)
}

/// Transactions in ledger order. Archived portfolios remain readable.
pub async fn list(
    db: &DatabaseConnection,
    owner: UserId,
    portfolio_id: Uuid,
    instrument_id: Option<Uuid>,
    page: Page,
) -> Result<Paged<LedgerEntry>, LedgerError> {
    let txn = db.begin().await?;
    if portfolios::find(&txn, owner, portfolio_id).await?.is_none() {
        return Err(LedgerError::NotFound);
    }
    let total = repository::count(&txn, owner, portfolio_id, instrument_id).await?;
    let models = repository::list(
        &txn,
        owner,
        portfolio_id,
        instrument_id,
        page.limit,
        page.offset,
    )
    .await?;
    txn.commit().await?;
    let items = models.into_iter().map(to_entry).collect::<Result<_, _>>()?;
    Ok(Paged { items, total, page })
}

/// Runs `body` in a transaction that starts by locking the owned, active
/// portfolio, and commits only if `body` succeeds.
async fn write<F, Fut>(
    db: &DatabaseConnection,
    owner: UserId,
    portfolio_id: Uuid,
    body: F,
) -> Result<LedgerEntry, LedgerError>
where
    F: FnOnce(DatabaseTransaction) -> Fut,
    Fut: Future<Output = Result<(DatabaseTransaction, transaction::Model), LedgerError>>,
{
    let txn = db.begin().await?;
    let locked =
        portfolios::lock_active(&txn, owner, portfolio_id, OffsetDateTime::now_utc()).await?;
    if locked == 0 {
        // Release the connection before asking why (the pool may hold one).
        txn.rollback().await?;
        return Err(match portfolios::find(db, owner, portfolio_id).await? {
            Some(_) => LedgerError::PortfolioArchived,
            None => LedgerError::NotFound,
        });
    }
    // On error the transaction is dropped, which rolls it back.
    let result = async {
        let (txn, model) = body(txn).await?;
        txn.commit().await?;
        Ok(model)
    }
    .await;
    match result {
        Ok(model) => to_entry(model),
        // A referenced instrument or cash account was deleted after it was
        // checked. SQLite serializes writers so this cannot happen there,
        // but other databases may let a concurrent delete win the race.
        Err(LedgerError::Database(err)) if is_foreign_key_violation(&err) => {
            Err(LedgerError::Validation(vec![FieldError {
                field: "body",
                message: "refers to an instrument or cash account that no longer exists".to_owned(),
            }]))
        }
        Err(err) => Err(err),
    }
}

/// The instrument must belong to the caller and the cash accounts to this
/// portfolio. A cash account used with an instrument must be in its quote
/// currency, and a transfer stays within one currency (FX is a separate
/// transaction kind). Missing and foreign references get the same message.
async fn check_references(
    txn: &DatabaseTransaction,
    owner: UserId,
    portfolio_id: Uuid,
    valid: &ValidTransaction,
) -> Result<(), LedgerError> {
    let mut errors = Vec::new();
    let instrument = match valid.instrument_id() {
        Some(id) => {
            let found = instruments::find(txn, owner, id).await?;
            if found.is_none() {
                errors.push(FieldError {
                    field: "instrument_id",
                    message: "does not refer to one of your instruments".to_owned(),
                });
            }
            found
        }
        None => None,
    };
    let mut accounts = Vec::new();
    for (field, id) in [
        ("cash_account_id", valid.cash_account_id()),
        (
            "destination_cash_account_id",
            valid.destination_cash_account_id(),
        ),
    ] {
        let Some(id) = id else { continue };
        match cash_accounts::find(txn, owner, portfolio_id, id).await? {
            None => errors.push(FieldError {
                field,
                message: "does not refer to a cash account in this portfolio".to_owned(),
            }),
            Some(account) => accounts.push((field, account)),
        }
    }
    match (accounts.as_slice(), &instrument) {
        ([(field, account)], Some(instrument)) if account.currency != instrument.quote_currency => {
            errors.push(FieldError {
                field,
                message: format!(
                    "is in {} but the instrument is quoted in {}",
                    account.currency, instrument.quote_currency
                ),
            });
        }
        ([(_, source), (field, destination)], _) => {
            let same = source.currency == destination.currency;
            if valid.kind == TransactionKind::FxExchange && same {
                errors.push(FieldError {
                    field,
                    message: format!(
                        "is in {}, the same currency as the source account; use CASH_TRANSFER",
                        destination.currency
                    ),
                });
            } else if valid.kind == TransactionKind::CashTransfer && !same {
                errors.push(FieldError {
                    field,
                    message: format!(
                        "is in {} but the source account is in {}; use FX_EXCHANGE",
                        destination.currency, source.currency
                    ),
                });
            }
        }
        _ => {}
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(LedgerError::Validation(errors))
    }
}

async fn post_movements(
    txn: &DatabaseTransaction,
    model: &transaction::Model,
    valid: &ValidTransaction,
) -> Result<(), LedgerError> {
    let effects = valid
        .effects()
        .map_err(|error| LedgerError::Validation(vec![error]))?;
    // Movement ids are time-ordered, so a transaction's movements replay in
    // the order they are posted here.
    for position in effects.position {
        repository::insert_position_movement(
            txn,
            position_movement::Model {
                id: Uuid::now_v7(),
                transaction_id: model.id,
                portfolio_id: model.portfolio_id,
                instrument_id: position.instrument_id,
                kind: position.kind.as_str().to_owned(),
                quantity: position.quantity.to_string(),
                amount: position.amount.to_string(),
            },
        )
        .await?;
    }
    for cash in effects.cash {
        repository::insert_cash_movement(
            txn,
            cash_movement::Model {
                id: Uuid::now_v7(),
                transaction_id: model.id,
                portfolio_id: model.portfolio_id,
                cash_account_id: cash.cash_account_id,
                amount_delta: cash.amount_delta.to_string(),
            },
        )
        .await?;
    }
    Ok(())
}

/// Replays FIFO for each instrument from its full history.
async fn replay(
    txn: &DatabaseTransaction,
    portfolio_id: Uuid,
    instruments: BTreeSet<Uuid>,
) -> Result<(), LedgerError> {
    for instrument_id in instruments {
        let history = repository::position_history(txn, portfolio_id, instrument_id).await?;
        let events = history
            .iter()
            .map(|(movement, _)| to_event(movement))
            .collect::<Result<Vec<_>, _>>()?;
        match fifo::replay(events) {
            Ok(_) => {}
            Err(FifoError::InsufficientHoldings { event, .. }) => {
                return Err(LedgerError::InsufficientHoldings {
                    instrument_id,
                    trade_date: history[event].1.trade_date,
                });
            }
            Err(FifoError::NoHoldings { event }) => {
                return Err(LedgerError::NoHoldings {
                    instrument_id,
                    trade_date: history[event].1.trade_date,
                });
            }
            Err(FifoError::Overflow { .. }) => {
                return Err(LedgerError::Validation(vec![FieldError {
                    field: "quantity",
                    message: "the resulting holdings or cost basis are too large".to_owned(),
                }]));
            }
            Err(FifoError::RoundsToZero { event }) => {
                return Err(LedgerError::Validation(vec![FieldError {
                    field: "ratio",
                    message: format!(
                        "the split on {} would leave a lot too small to represent",
                        history[event].1.trade_date
                    ),
                }]));
            }
        }
    }
    Ok(())
}

pub(crate) fn to_event(movement: &position_movement::Model) -> Result<Event, DbErr> {
    let corrupt = |err: String| DbErr::Type(format!("movement {}: {err}", movement.id));
    let quantity = Quantity::from_stored(&movement.quantity).map_err(corrupt)?;
    match MovementKind::from_db(&movement.kind) {
        Some(MovementKind::Acquire) => Ok(Event::Acquire {
            quantity,
            cost: Amount::from_stored(&movement.amount).map_err(corrupt)?,
        }),
        Some(MovementKind::Dispose) => Ok(Event::Dispose {
            quantity,
            proceeds: SignedAmount::from_stored(&movement.amount).map_err(corrupt)?,
        }),
        Some(MovementKind::Split) => Ok(Event::Split {
            ratio: Ratio::from_parts(
                quantity,
                Quantity::from_stored(&movement.amount).map_err(corrupt)?,
            ),
        }),
        Some(MovementKind::StockDividend) => Ok(Event::StockDividend { quantity }),
        None => Err(corrupt(format!(
            "unknown movement kind {:?}",
            movement.kind
        ))),
    }
}

fn to_model(
    id: Uuid,
    portfolio_id: Uuid,
    valid: &ValidTransaction,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
) -> transaction::Model {
    let text = |value: &dyn ToString| Some(value.to_string());
    let mut model = transaction::Model {
        id,
        portfolio_id,
        kind: valid.kind.as_str().to_owned(),
        trade_date: valid.trade_date,
        sort_order: valid.sort_order,
        instrument_id: valid.instrument_id(),
        cash_account_id: valid.cash_account_id(),
        destination_cash_account_id: valid.destination_cash_account_id(),
        quantity: None,
        ratio: None,
        price: None,
        fee: None,
        tax: None,
        amount: None,
        destination_amount: None,
        cash_in_lieu_quantity: None,
        note: valid.note.clone(),
        created_at,
        updated_at,
    };
    match &valid.details {
        Details::OpeningPosition {
            quantity,
            cost_basis,
            ..
        } => {
            model.quantity = text(quantity);
            model.amount = text(cost_basis);
        }
        Details::OpeningCash { amount, .. } => model.amount = text(amount),
        Details::CashFlow { amount, .. } => model.amount = text(amount),
        Details::CashTransfer { amount, fee, .. } => {
            model.amount = text(amount);
            model.fee = text(fee);
        }
        Details::FxExchange {
            amount,
            destination_amount,
            fee,
            ..
        } => {
            model.amount = text(amount);
            model.destination_amount = text(destination_amount);
            model.fee = text(fee);
        }
        Details::StockSplit {
            ratio,
            cash_in_lieu,
            ..
        } => {
            model.ratio = text(ratio);
            if let Some(paid) = cash_in_lieu {
                model.cash_in_lieu_quantity = text(&paid.quantity);
                model.amount = text(&paid.amount);
            }
        }
        Details::StockDividend {
            quantity,
            cash_in_lieu,
            ..
        } => {
            model.quantity = text(quantity);
            if let Some(paid) = cash_in_lieu {
                model.cash_in_lieu_quantity = text(&paid.quantity);
                model.amount = text(&paid.amount);
            }
        }
        Details::Income {
            gross, fee, tax, ..
        } => {
            model.amount = text(gross);
            model.fee = text(fee);
            model.tax = text(tax);
        }
        Details::Trade {
            quantity,
            price,
            fee,
            tax,
            ..
        } => {
            model.quantity = text(quantity);
            model.price = text(price);
            model.fee = text(fee);
            model.tax = text(tax);
        }
    }
    model
}

fn to_entry(model: transaction::Model) -> Result<LedgerEntry, LedgerError> {
    Ok(LedgerEntry {
        id: model.id,
        portfolio_id: model.portfolio_id,
        kind: TransactionKind::from_db(&model.kind)?,
        trade_date: model.trade_date,
        sort_order: model.sort_order,
        instrument_id: model.instrument_id,
        cash_account_id: model.cash_account_id,
        destination_cash_account_id: model.destination_cash_account_id,
        quantity: model.quantity,
        ratio: model.ratio,
        price: model.price,
        fee: model.fee,
        tax: model.tax,
        amount: model.amount,
        destination_amount: model.destination_amount,
        cash_in_lieu_quantity: model.cash_in_lieu_quantity,
        note: model.note,
        created_at: model.created_at,
        updated_at: model.updated_at,
    })
}
