//! Transaction kinds, input validation and their effects on positions and
//! cash (ADR-LEDG-04 … LEDG-07, ADR-TXN-01 … TXN-08).

use rust_decimal::Decimal;
use sea_orm::DbErr;
use time::Date;
use time::macros::{date, format_description};
use uuid::Uuid;

use super::decimal::{
    Amount, Price, Quantity, Ratio, SignedAmount, exact_add, exact_mul, exact_sub,
};
use crate::auth::FieldError;

pub const MAX_NOTE_CHARS: usize = 500;
const EARLIEST_DATE: Date = date!(1900 - 01 - 01);
const LATEST_DATE: Date = date!(2100 - 12 - 31);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionKind {
    /// One FIFO lot carried over from before tracking started; no cash.
    OpeningPosition,
    /// A starting cash balance for one cash account (may be negative).
    OpeningCash,
    Buy,
    Sell,
    /// Money paid into a cash account.
    Deposit,
    /// Money taken out of a cash account.
    Withdrawal,
    /// A dividend or fund distribution paid by an instrument.
    Dividend,
    /// Interest earned on a cash account.
    Interest,
    /// A standalone charge on a cash account.
    Fee,
    /// A standalone tax payment from a cash account.
    Tax,
    /// A refund of tax into a cash account.
    TaxReturn,
    /// Money moved between two cash accounts in the same currency.
    CashTransfer,
    /// One currency bought with another, between two cash accounts.
    FxExchange,
    /// A split or reverse split of an instrument's shares.
    StockSplit,
    /// New shares of an instrument received as a dividend.
    StockDividend,
}

impl TransactionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpeningPosition => "OPENING_POSITION",
            Self::OpeningCash => "OPENING_CASH",
            Self::Buy => "BUY",
            Self::Sell => "SELL",
            Self::Deposit => "DEPOSIT",
            Self::Withdrawal => "WITHDRAWAL",
            Self::Dividend => "DIVIDEND",
            Self::Interest => "INTEREST",
            Self::Fee => "FEE",
            Self::Tax => "TAX",
            Self::TaxReturn => "TAX_RETURN",
            Self::CashTransfer => "CASH_TRANSFER",
            Self::FxExchange => "FX_EXCHANGE",
            Self::StockSplit => "STOCK_SPLIT",
            Self::StockDividend => "STOCK_DIVIDEND",
        }
    }

    pub fn from_db(value: &str) -> Result<Self, DbErr> {
        match value {
            "OPENING_POSITION" => Ok(Self::OpeningPosition),
            "OPENING_CASH" => Ok(Self::OpeningCash),
            "BUY" => Ok(Self::Buy),
            "SELL" => Ok(Self::Sell),
            "DEPOSIT" => Ok(Self::Deposit),
            "WITHDRAWAL" => Ok(Self::Withdrawal),
            "DIVIDEND" => Ok(Self::Dividend),
            "INTEREST" => Ok(Self::Interest),
            "FEE" => Ok(Self::Fee),
            "TAX" => Ok(Self::Tax),
            "TAX_RETURN" => Ok(Self::TaxReturn),
            "CASH_TRANSFER" => Ok(Self::CashTransfer),
            "FX_EXCHANGE" => Ok(Self::FxExchange),
            "STOCK_SPLIT" => Ok(Self::StockSplit),
            "STOCK_DIVIDEND" => Ok(Self::StockDividend),
            other => Err(DbErr::Type(format!("unknown transaction kind {other:?}"))),
        }
    }

    /// The optional input fields this kind accepts, besides `trade_date`,
    /// `sort_order` and `note`. Any other field is rejected.
    fn accepts(self) -> &'static [&'static str] {
        match self {
            Self::OpeningPosition => &["instrument_id", "quantity", "amount"],
            Self::OpeningCash
            | Self::Deposit
            | Self::Withdrawal
            | Self::Fee
            | Self::Tax
            | Self::TaxReturn => &["cash_account_id", "amount"],
            Self::Buy | Self::Sell => &[
                "instrument_id",
                "cash_account_id",
                "quantity",
                "price",
                "fee",
                "tax",
            ],
            Self::Dividend => &["instrument_id", "cash_account_id", "amount", "fee", "tax"],
            Self::Interest => &["cash_account_id", "amount", "fee", "tax"],
            Self::CashTransfer => &[
                "cash_account_id",
                "destination_cash_account_id",
                "amount",
                "fee",
            ],
            Self::FxExchange => &[
                "cash_account_id",
                "destination_cash_account_id",
                "amount",
                "destination_amount",
                "fee",
            ],
            Self::StockSplit => &[
                "instrument_id",
                "ratio",
                "cash_in_lieu_quantity",
                "amount",
                "cash_account_id",
            ],
            Self::StockDividend => &[
                "instrument_id",
                "quantity",
                "cash_in_lieu_quantity",
                "amount",
                "cash_account_id",
            ],
        }
    }
}

/// Raw, unvalidated fields as received from a client.
#[derive(Debug, Clone, Default)]
pub struct RawTransaction<'a> {
    pub trade_date: &'a str,
    pub sort_order: Option<i32>,
    pub instrument_id: Option<Uuid>,
    pub cash_account_id: Option<Uuid>,
    pub destination_cash_account_id: Option<Uuid>,
    pub quantity: Option<&'a str>,
    pub ratio: Option<&'a str>,
    pub price: Option<&'a str>,
    pub fee: Option<&'a str>,
    pub tax: Option<&'a str>,
    pub amount: Option<&'a str>,
    pub destination_amount: Option<&'a str>,
    pub cash_in_lieu_quantity: Option<&'a str>,
    pub note: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Details {
    OpeningPosition {
        instrument_id: Uuid,
        quantity: Quantity,
        cost_basis: Amount,
    },
    OpeningCash {
        cash_account_id: Uuid,
        amount: SignedAmount,
    },
    /// A buy or sell; fees and taxes are in the instrument's quote currency.
    Trade {
        instrument_id: Uuid,
        cash_account_id: Option<Uuid>,
        quantity: Quantity,
        price: Price,
        fee: Amount,
        tax: Amount,
    },
    /// A deposit, withdrawal, fee, tax or tax return: one positive amount on
    /// one account. The kind gives the direction.
    CashFlow {
        cash_account_id: Uuid,
        amount: Amount,
    },
    /// A dividend (from an instrument) or interest (on a cash account):
    /// the gross amount, less any fee and tax withheld (ADR-TXN-05).
    Income {
        instrument_id: Option<Uuid>,
        cash_account_id: Option<Uuid>,
        gross: Amount,
        fee: Amount,
        tax: Amount,
    },
    /// Moves `amount` from one account to another; the source also pays
    /// `fee` (ADR-TXN-03/04).
    CashTransfer {
        cash_account_id: Uuid,
        destination_cash_account_id: Uuid,
        amount: Amount,
        fee: Amount,
    },
    /// Spends `amount` (plus `fee`) from the source account and receives
    /// `destination_amount` in an account in another currency (ADR-TXN-04).
    /// The executed rate is the ratio of the two amounts.
    FxExchange {
        cash_account_id: Uuid,
        amount: Amount,
        destination_cash_account_id: Uuid,
        destination_amount: Amount,
        fee: Amount,
    },
    StockSplit {
        instrument_id: Uuid,
        ratio: Ratio,
        cash_in_lieu: Option<CashInLieu>,
    },
    /// `quantity` new shares spread over the open lots at no cost.
    StockDividend {
        instrument_id: Uuid,
        quantity: Quantity,
        cash_in_lieu: Option<CashInLieu>,
    },
}

/// Fractional shares a corporate action pays out in cash instead: the
/// `quantity` (in post-action shares) is disposed of through FIFO for
/// `amount`, realizing profit or loss (ADR-TXN-06).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CashInLieu {
    pub quantity: Quantity,
    pub amount: Amount,
    pub cash_account_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidTransaction {
    pub kind: TransactionKind,
    pub trade_date: Date,
    pub sort_order: i32,
    pub details: Details,
    pub note: Option<String>,
}

/// How a position movement changes a holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovementKind {
    /// Opens a lot of `quantity` costing `amount`.
    Acquire,
    /// Disposes of `quantity` for net proceeds `amount`.
    Dispose,
    /// Multiplies the holding by `quantity / amount` (new : old shares).
    Split,
    /// Adds `quantity` shares over the open lots; `amount` is zero.
    StockDividend,
}

impl MovementKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Acquire => "ACQUIRE",
            Self::Dispose => "DISPOSE",
            Self::Split => "SPLIT",
            Self::StockDividend => "STOCK_DIVIDEND",
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "ACQUIRE" => Some(Self::Acquire),
            "DISPOSE" => Some(Self::Dispose),
            "SPLIT" => Some(Self::Split),
            "STOCK_DIVIDEND" => Some(Self::StockDividend),
            _ => None,
        }
    }
}

/// One change to the holding of one instrument; see [`MovementKind`] for
/// what `quantity` and `amount` mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PositionEffect {
    pub instrument_id: Uuid,
    pub kind: MovementKind,
    pub quantity: Decimal,
    pub amount: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CashEffect {
    pub cash_account_id: Uuid,
    pub amount_delta: Decimal,
}

/// The movements of one transaction, in the order they apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effects {
    pub position: Vec<PositionEffect>,
    pub cash: Vec<CashEffect>,
}

pub fn parse_date(input: &str) -> Result<Date, String> {
    let date = Date::parse(input.trim(), format_description!("[year]-[month]-[day]"))
        .map_err(|_| "must be a date in YYYY-MM-DD format".to_owned())?;
    if !(EARLIEST_DATE..=LATEST_DATE).contains(&date) {
        return Err("must be between 1900-01-01 and 2100-12-31".to_owned());
    }
    Ok(date)
}

/// Checks the fields required and allowed for `kind` and parses them.
pub fn validate(
    kind: TransactionKind,
    raw: &RawTransaction<'_>,
) -> Result<ValidTransaction, Vec<FieldError>> {
    let mut v = Validator::default();
    let trade_date = v.field("trade_date", parse_date(raw.trade_date));
    let note = raw
        .note
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .and_then(|note| {
            let checked = if note.chars().count() > MAX_NOTE_CHARS {
                Err(format!("must be at most {MAX_NOTE_CHARS} characters"))
            } else if note
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')
            {
                Err("must not contain control characters".to_owned())
            } else {
                Ok(note.to_owned())
            };
            v.field("note", checked)
        });

    let present = [
        ("instrument_id", raw.instrument_id.is_some()),
        ("cash_account_id", raw.cash_account_id.is_some()),
        (
            "destination_cash_account_id",
            raw.destination_cash_account_id.is_some(),
        ),
        ("quantity", raw.quantity.is_some()),
        ("ratio", raw.ratio.is_some()),
        ("price", raw.price.is_some()),
        ("fee", raw.fee.is_some()),
        ("tax", raw.tax.is_some()),
        ("amount", raw.amount.is_some()),
        ("destination_amount", raw.destination_amount.is_some()),
        ("cash_in_lieu_quantity", raw.cash_in_lieu_quantity.is_some()),
    ];
    for (field, is_present) in present {
        v.forbid(field, is_present && !kind.accepts().contains(&field), kind);
    }

    let details = match kind {
        TransactionKind::OpeningPosition => {
            let instrument_id = v.require("instrument_id", raw.instrument_id, kind);
            let quantity = v.parse("quantity", raw.quantity, kind, Quantity::parse);
            let cost_basis = v.parse("amount", raw.amount, kind, Amount::parse);
            match (instrument_id, quantity, cost_basis) {
                (Some(instrument_id), Some(quantity), Some(cost_basis)) => {
                    Some(Details::OpeningPosition {
                        instrument_id,
                        quantity,
                        cost_basis,
                    })
                }
                _ => None,
            }
        }
        TransactionKind::OpeningCash => {
            let cash_account_id = v.require("cash_account_id", raw.cash_account_id, kind);
            let amount = v.parse("amount", raw.amount, kind, SignedAmount::parse);
            match (cash_account_id, amount) {
                (Some(cash_account_id), Some(amount)) => Some(Details::OpeningCash {
                    cash_account_id,
                    amount,
                }),
                _ => None,
            }
        }
        TransactionKind::Buy | TransactionKind::Sell => {
            let instrument_id = v.require("instrument_id", raw.instrument_id, kind);
            let quantity = v.parse("quantity", raw.quantity, kind, Quantity::parse);
            let price = v.parse("price", raw.price, kind, Price::parse);
            let fee = v.optional("fee", raw.fee);
            let tax = v.optional("tax", raw.tax);
            match (instrument_id, quantity, price, fee, tax) {
                (Some(instrument_id), Some(quantity), Some(price), Some(fee), Some(tax)) => {
                    Some(Details::Trade {
                        instrument_id,
                        cash_account_id: raw.cash_account_id,
                        quantity,
                        price,
                        fee,
                        tax,
                    })
                }
                _ => None,
            }
        }
        TransactionKind::Deposit
        | TransactionKind::Withdrawal
        | TransactionKind::Fee
        | TransactionKind::Tax
        | TransactionKind::TaxReturn => {
            let cash_account_id = v.require("cash_account_id", raw.cash_account_id, kind);
            let amount = v.parse("amount", raw.amount, kind, positive_amount);
            match (cash_account_id, amount) {
                (Some(cash_account_id), Some(amount)) => Some(Details::CashFlow {
                    cash_account_id,
                    amount,
                }),
                _ => None,
            }
        }
        TransactionKind::Dividend | TransactionKind::Interest => {
            // A dividend comes from an instrument and may skip the cash
            // account, like a trade; interest is earned on a cash account.
            let instrument_id = if kind == TransactionKind::Dividend {
                v.require("instrument_id", raw.instrument_id, kind)
                    .map(Some)
            } else {
                Some(None)
            };
            let cash_account_id = if kind == TransactionKind::Interest {
                v.require("cash_account_id", raw.cash_account_id, kind)
                    .map(Some)
            } else {
                Some(raw.cash_account_id)
            };
            let gross = v.parse("amount", raw.amount, kind, positive_amount);
            let fee = v.optional("fee", raw.fee);
            let tax = v.optional("tax", raw.tax);
            match (instrument_id, cash_account_id, gross, fee, tax) {
                (Some(instrument_id), Some(cash_account_id), Some(gross), Some(fee), Some(tax)) => {
                    Some(Details::Income {
                        instrument_id,
                        cash_account_id,
                        gross,
                        fee,
                        tax,
                    })
                }
                _ => None,
            }
        }
        TransactionKind::CashTransfer | TransactionKind::FxExchange => {
            let cash_account_id = v.require("cash_account_id", raw.cash_account_id, kind);
            let destination = v.require(
                "destination_cash_account_id",
                raw.destination_cash_account_id,
                kind,
            );
            let amount = v.parse("amount", raw.amount, kind, positive_amount);
            let destination_amount = if kind == TransactionKind::FxExchange {
                v.parse(
                    "destination_amount",
                    raw.destination_amount,
                    kind,
                    positive_amount,
                )
            } else {
                Some(Amount::default())
            };
            let fee = v.optional("fee", raw.fee);
            if cash_account_id.is_some() && cash_account_id == destination {
                v.field::<()>(
                    "destination_cash_account_id",
                    Err("must differ from cash_account_id".to_owned()),
                );
            }
            match (
                cash_account_id,
                destination,
                amount,
                destination_amount,
                fee,
            ) {
                (
                    Some(cash_account_id),
                    Some(destination_cash_account_id),
                    Some(amount),
                    Some(destination_amount),
                    Some(fee),
                ) => Some(if kind == TransactionKind::CashTransfer {
                    Details::CashTransfer {
                        cash_account_id,
                        destination_cash_account_id,
                        amount,
                        fee,
                    }
                } else {
                    Details::FxExchange {
                        cash_account_id,
                        amount,
                        destination_cash_account_id,
                        destination_amount,
                        fee,
                    }
                }),
                _ => None,
            }
        }
        TransactionKind::StockSplit => {
            let instrument_id = v.require("instrument_id", raw.instrument_id, kind);
            let ratio = v.parse("ratio", raw.ratio, kind, Ratio::parse);
            let cash_in_lieu = v.cash_in_lieu(raw);
            match (instrument_id, ratio, cash_in_lieu) {
                (Some(instrument_id), Some(ratio), Some(cash_in_lieu)) => {
                    Some(Details::StockSplit {
                        instrument_id,
                        ratio,
                        cash_in_lieu,
                    })
                }
                _ => None,
            }
        }
        TransactionKind::StockDividend => {
            let instrument_id = v.require("instrument_id", raw.instrument_id, kind);
            let quantity = v.parse("quantity", raw.quantity, kind, Quantity::parse);
            let cash_in_lieu = v.cash_in_lieu(raw);
            match (instrument_id, quantity, cash_in_lieu) {
                (Some(instrument_id), Some(quantity), Some(cash_in_lieu)) => {
                    Some(Details::StockDividend {
                        instrument_id,
                        quantity,
                        cash_in_lieu,
                    })
                }
                _ => None,
            }
        }
    };

    match (trade_date, details) {
        (Some(trade_date), Some(details)) if v.errors.is_empty() => {
            let valid = ValidTransaction {
                kind,
                trade_date,
                sort_order: raw.sort_order.unwrap_or(0),
                details,
                note,
            };
            // Reject values whose derived amounts are invalid up front.
            valid.effects().map_err(|error| vec![error])?;
            Ok(valid)
        }
        _ => Err(v.errors),
    }
}

/// A non-negative amount that must also be non-zero.
fn positive_amount(input: &str) -> Result<Amount, String> {
    let amount = Amount::parse(input)?;
    if amount.value().is_zero() {
        return Err("must be greater than zero".to_owned());
    }
    Ok(amount)
}

impl ValidTransaction {
    pub fn instrument_id(&self) -> Option<Uuid> {
        match self.details {
            Details::OpeningPosition { instrument_id, .. }
            | Details::Trade { instrument_id, .. }
            | Details::StockSplit { instrument_id, .. }
            | Details::StockDividend { instrument_id, .. } => Some(instrument_id),
            Details::Income { instrument_id, .. } => instrument_id,
            Details::OpeningCash { .. }
            | Details::CashFlow { .. }
            | Details::CashTransfer { .. }
            | Details::FxExchange { .. } => None,
        }
    }

    pub fn cash_account_id(&self) -> Option<Uuid> {
        match self.details {
            Details::OpeningCash {
                cash_account_id, ..
            }
            | Details::CashFlow {
                cash_account_id, ..
            }
            | Details::CashTransfer {
                cash_account_id, ..
            }
            | Details::FxExchange {
                cash_account_id, ..
            } => Some(cash_account_id),
            Details::Trade {
                cash_account_id, ..
            }
            | Details::Income {
                cash_account_id, ..
            } => cash_account_id,
            Details::StockSplit { cash_in_lieu, .. }
            | Details::StockDividend { cash_in_lieu, .. } => {
                cash_in_lieu.and_then(|c| c.cash_account_id)
            }
            Details::OpeningPosition { .. } => None,
        }
    }

    pub fn destination_cash_account_id(&self) -> Option<Uuid> {
        match self.details {
            Details::CashTransfer {
                destination_cash_account_id,
                ..
            }
            | Details::FxExchange {
                destination_cash_account_id,
                ..
            } => Some(destination_cash_account_id),
            _ => None,
        }
    }

    /// The movements this transaction posts. All arithmetic is exact:
    /// a result that would overflow or need rounding is rejected.
    ///
    /// - Buy: cost = quantity × price + fee + tax; cash −cost.
    /// - Sell: proceeds = quantity × price − fee − tax; cash +proceeds.
    /// - Deposit, tax return: cash +amount. Withdrawal, fee, tax: cash −amount.
    /// - Dividend, interest: net = amount − fee − tax, which must not be
    ///   negative; cash +net.
    /// - Cash transfer: source −(amount + fee), destination +amount.
    /// - FX exchange: source −(amount + fee), destination +destination_amount.
    /// - Stock split, stock dividend: the lot adjustment, then any cash in
    ///   lieu as a disposal; cash +amount.
    pub fn effects(&self) -> Result<Effects, FieldError> {
        let overflow = |field| FieldError {
            field,
            message: "the resulting amount is too large or too precise to be exact".to_owned(),
        };
        let cash = |cash_account_id, amount_delta| CashEffect {
            cash_account_id,
            amount_delta,
        };
        let position = |instrument_id, kind, quantity, amount| PositionEffect {
            instrument_id,
            kind,
            quantity,
            amount,
        };
        Ok(match self.details {
            Details::OpeningPosition {
                instrument_id,
                quantity,
                cost_basis,
            } => Effects {
                position: vec![position(
                    instrument_id,
                    MovementKind::Acquire,
                    quantity.value(),
                    cost_basis.value(),
                )],
                cash: Vec::new(),
            },
            Details::OpeningCash {
                cash_account_id,
                amount,
            } => Effects {
                position: Vec::new(),
                cash: vec![cash(cash_account_id, amount.value())],
            },
            Details::Trade {
                instrument_id,
                cash_account_id,
                quantity,
                price,
                fee,
                tax,
            } => {
                let gross = exact_mul(quantity.value(), price.value())
                    .ok_or_else(|| overflow("quantity"))?;
                let charges =
                    exact_add(fee.value(), tax.value()).ok_or_else(|| overflow("quantity"))?;
                let (kind, amount, cash_delta) = if self.kind == TransactionKind::Buy {
                    let cost = exact_add(gross, charges).ok_or_else(|| overflow("quantity"))?;
                    (MovementKind::Acquire, cost, -cost)
                } else {
                    let proceeds = exact_sub(gross, charges).ok_or_else(|| overflow("quantity"))?;
                    (MovementKind::Dispose, proceeds, proceeds)
                };
                Effects {
                    position: vec![position(instrument_id, kind, quantity.value(), amount)],
                    cash: cash_account_id
                        .map(|id| cash(id, cash_delta))
                        .into_iter()
                        .collect(),
                }
            }
            Details::CashFlow {
                cash_account_id,
                amount,
            } => {
                let credit = matches!(
                    self.kind,
                    TransactionKind::Deposit | TransactionKind::TaxReturn
                );
                let delta = if credit {
                    amount.value()
                } else {
                    -amount.value()
                };
                Effects {
                    position: Vec::new(),
                    cash: vec![cash(cash_account_id, delta)],
                }
            }
            Details::Income {
                cash_account_id,
                gross,
                fee,
                tax,
                ..
            } => {
                let charges =
                    exact_add(fee.value(), tax.value()).ok_or_else(|| overflow("amount"))?;
                let net = exact_sub(gross.value(), charges).ok_or_else(|| overflow("amount"))?;
                if net < Decimal::ZERO {
                    return Err(FieldError {
                        field: "amount",
                        message: "must be at least fee plus tax".to_owned(),
                    });
                }
                Effects {
                    position: Vec::new(),
                    cash: cash_account_id
                        .map(|id| cash(id, net))
                        .into_iter()
                        .collect(),
                }
            }
            Details::CashTransfer {
                cash_account_id,
                destination_cash_account_id,
                amount,
                fee,
            } => {
                let debit =
                    exact_add(amount.value(), fee.value()).ok_or_else(|| overflow("amount"))?;
                Effects {
                    position: Vec::new(),
                    cash: vec![
                        cash(cash_account_id, -debit),
                        cash(destination_cash_account_id, amount.value()),
                    ],
                }
            }
            Details::FxExchange {
                cash_account_id,
                amount,
                destination_cash_account_id,
                destination_amount,
                fee,
            } => {
                let debit =
                    exact_add(amount.value(), fee.value()).ok_or_else(|| overflow("amount"))?;
                Effects {
                    position: Vec::new(),
                    cash: vec![
                        cash(cash_account_id, -debit),
                        cash(destination_cash_account_id, destination_amount.value()),
                    ],
                }
            }
            Details::StockSplit {
                instrument_id,
                ratio,
                cash_in_lieu,
            } => corporate_action(
                position(
                    instrument_id,
                    MovementKind::Split,
                    ratio.new_shares().value(),
                    ratio.old_shares().value(),
                ),
                cash_in_lieu,
            ),
            Details::StockDividend {
                instrument_id,
                quantity,
                cash_in_lieu,
            } => corporate_action(
                position(
                    instrument_id,
                    MovementKind::StockDividend,
                    quantity.value(),
                    Decimal::ZERO,
                ),
                cash_in_lieu,
            ),
        })
    }
}

/// A lot adjustment followed by any cash in lieu, disposed of through FIFO.
fn corporate_action(adjustment: PositionEffect, cash_in_lieu: Option<CashInLieu>) -> Effects {
    let mut effects = Effects {
        position: vec![adjustment],
        cash: Vec::new(),
    };
    if let Some(paid) = cash_in_lieu {
        effects.position.push(PositionEffect {
            instrument_id: adjustment.instrument_id,
            kind: MovementKind::Dispose,
            quantity: paid.quantity.value(),
            amount: paid.amount.value(),
        });
        if let Some(cash_account_id) = paid.cash_account_id {
            effects.cash.push(CashEffect {
                cash_account_id,
                amount_delta: paid.amount.value(),
            });
        }
    }
    effects
}

#[derive(Default)]
struct Validator {
    errors: Vec<FieldError>,
}

impl Validator {
    fn field<T>(&mut self, field: &'static str, result: Result<T, String>) -> Option<T> {
        result
            .map_err(|message| self.errors.push(FieldError { field, message }))
            .ok()
    }

    fn forbid(&mut self, field: &'static str, present: bool, kind: TransactionKind) {
        if present {
            self.errors.push(FieldError {
                field,
                message: format!("is not allowed for {}", kind.as_str()),
            });
        }
    }

    fn require<T>(
        &mut self,
        field: &'static str,
        value: Option<T>,
        kind: TransactionKind,
    ) -> Option<T> {
        if value.is_none() {
            self.errors.push(FieldError {
                field,
                message: format!("is required for {}", kind.as_str()),
            });
        }
        value
    }

    fn parse<T>(
        &mut self,
        field: &'static str,
        value: Option<&str>,
        kind: TransactionKind,
        parse: impl Fn(&str) -> Result<T, String>,
    ) -> Option<T> {
        let text = self.require(field, value, kind)?;
        self.field(field, parse(text))
    }

    /// An optional non-negative amount that defaults to zero.
    fn optional(&mut self, field: &'static str, value: Option<&str>) -> Option<Amount> {
        match value {
            None => Some(Amount::default()),
            Some(text) => self.field(field, Amount::parse(text)),
        }
    }

    /// Optional cash in lieu: `cash_in_lieu_quantity` and `amount` come
    /// together, and `cash_account_id` is only allowed with them.
    /// `Some(None)` means none was given; `None` means it was invalid.
    fn cash_in_lieu(&mut self, raw: &RawTransaction<'_>) -> Option<Option<CashInLieu>> {
        let paired = |field| Err(format!("is required with {field}"));
        match (raw.cash_in_lieu_quantity, raw.amount) {
            (None, None) => {
                if raw.cash_account_id.is_some() {
                    self.field::<()>(
                        "cash_account_id",
                        Err("is only allowed with cash in lieu".to_owned()),
                    );
                    return None;
                }
                Some(None)
            }
            (Some(_), None) => self.field("amount", paired("cash_in_lieu_quantity")),
            (None, Some(_)) => self.field("cash_in_lieu_quantity", paired("amount")),
            (Some(quantity), Some(amount)) => {
                let quantity = self.field("cash_in_lieu_quantity", Quantity::parse(quantity));
                let amount = self.field("amount", Amount::parse(amount));
                Some(Some(CashInLieu {
                    quantity: quantity?,
                    amount: amount?,
                    cash_account_id: raw.cash_account_id,
                }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trade(kind: TransactionKind, qty: &str, price: &str, fee: &str, tax: &str) -> Effects {
        let raw = RawTransaction {
            trade_date: "2026-01-15",
            instrument_id: Some(Uuid::nil()),
            cash_account_id: Some(Uuid::max()),
            quantity: Some(qty),
            price: Some(price),
            fee: Some(fee),
            tax: Some(tax),
            ..Default::default()
        };
        validate(kind, &raw).unwrap().effects().unwrap()
    }

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn fields(errors: &[FieldError]) -> Vec<&'static str> {
        errors.iter().map(|e| e.field).collect()
    }

    #[test]
    fn buy_cost_includes_fees_and_taxes() {
        let effects = trade(TransactionKind::Buy, "100", "35.25", "15.50", "1.09");

        let position = effects.position[0];
        assert_eq!(position.kind, MovementKind::Acquire);
        assert_eq!(position.quantity, d("100"));
        assert_eq!(position.amount, d("3541.59")); // 100 × 35.25 = 3525; + 15.50 + 1.09
        assert_eq!(effects.cash[0].amount_delta, d("-3541.59"));
    }

    #[test]
    fn sell_proceeds_are_net_of_fees_and_taxes() {
        let effects = trade(TransactionKind::Sell, "40", "50", "12", "0.84");

        let position = effects.position[0];
        assert_eq!(position.kind, MovementKind::Dispose);
        assert_eq!(position.quantity, d("40"));
        assert_eq!(position.amount, d("1987.16"));
        assert_eq!(effects.cash[0].amount_delta, d("1987.16"));
    }

    #[test]
    fn products_are_exact() {
        let effects = trade(TransactionKind::Buy, "0.33333333", "3.00000001", "0", "0");

        // 0.33333333 × 3.00000001 = 0.99999999 + 0.0000000033333333
        assert_eq!(effects.position[0].amount, d("0.9999999933333333"));
    }

    #[test]
    fn overflowing_trades_are_rejected() {
        let raw = RawTransaction {
            trade_date: "2026-01-15",
            instrument_id: Some(Uuid::nil()),
            quantity: Some("999999999999999"),
            price: Some("999999999999999"),
            ..Default::default()
        };

        let errors = validate(TransactionKind::Buy, &raw).unwrap_err();
        assert_eq!(
            errors[0].message,
            "the resulting amount is too large or too precise to be exact"
        );
    }

    #[test]
    fn products_that_would_be_rounded_are_rejected() {
        let raw = RawTransaction {
            trade_date: "2026-01-15",
            instrument_id: Some(Uuid::nil()),
            quantity: Some("10000000.12345678"),
            price: Some("1000000.12345678"),
            ..Default::default()
        };

        assert!(validate(TransactionKind::Buy, &raw).is_err());
    }

    #[test]
    fn notes_reject_control_characters_but_allow_line_breaks() {
        let mut raw = RawTransaction {
            trade_date: "2026-01-01",
            cash_account_id: Some(Uuid::nil()),
            amount: Some("1"),
            note: Some("line one\nline\ttwo"),
            ..Default::default()
        };
        assert!(validate(TransactionKind::OpeningCash, &raw).is_ok());
        raw.note = Some("bad\u{0}note");
        assert_eq!(
            validate(TransactionKind::OpeningCash, &raw).unwrap_err()[0].field,
            "note"
        );
    }

    #[test]
    fn fields_are_checked_per_kind() {
        let raw = RawTransaction {
            trade_date: "2026-02-30",
            cash_account_id: Some(Uuid::nil()),
            price: Some("1"),
            ..Default::default()
        };

        let errors = validate(TransactionKind::OpeningPosition, &raw).unwrap_err();
        assert_eq!(
            fields(&errors),
            [
                "trade_date",
                "cash_account_id",
                "price",
                "instrument_id",
                "quantity",
                "amount"
            ]
        );
    }

    #[test]
    fn opening_cash_may_be_negative_and_trades_need_no_cash_account() {
        let raw = RawTransaction {
            trade_date: "2026-01-01",
            cash_account_id: Some(Uuid::nil()),
            amount: Some("-250.00"),
            ..Default::default()
        };
        let cash = validate(TransactionKind::OpeningCash, &raw).unwrap();
        assert_eq!(cash.effects().unwrap().cash[0].amount_delta, d("-250.00"));

        let raw = RawTransaction {
            trade_date: "2026-01-01",
            instrument_id: Some(Uuid::nil()),
            quantity: Some("1"),
            price: Some("10"),
            ..Default::default()
        };
        let buy = validate(TransactionKind::Buy, &raw).unwrap();
        assert!(buy.effects().unwrap().cash.is_empty());
    }

    #[test]
    fn cash_flows_move_one_account_in_the_kinds_direction() {
        let account = Uuid::max();
        for (kind, delta) in [
            (TransactionKind::Deposit, "1000.50"),
            (TransactionKind::TaxReturn, "1000.50"),
            (TransactionKind::Withdrawal, "-1000.50"),
            (TransactionKind::Fee, "-1000.50"),
            (TransactionKind::Tax, "-1000.50"),
        ] {
            let raw = RawTransaction {
                trade_date: "2026-03-01",
                cash_account_id: Some(account),
                amount: Some("1000.50"),
                ..Default::default()
            };
            let effects = validate(kind, &raw).unwrap().effects().unwrap();
            assert!(effects.position.is_empty(), "{kind:?}");
            assert_eq!(
                effects.cash,
                [CashEffect {
                    cash_account_id: account,
                    amount_delta: d(delta),
                }]
            );
        }
    }

    #[test]
    fn cash_flows_need_a_positive_amount_and_an_account() {
        for amount in ["0", "0.00", "-5"] {
            let raw = RawTransaction {
                trade_date: "2026-03-01",
                cash_account_id: Some(Uuid::nil()),
                amount: Some(amount),
                ..Default::default()
            };
            let errors = validate(TransactionKind::Deposit, &raw).unwrap_err();
            assert_eq!(fields(&errors), ["amount"], "{amount}");
        }

        let raw = RawTransaction {
            trade_date: "2026-03-01",
            instrument_id: Some(Uuid::nil()),
            fee: Some("1"),
            ..Default::default()
        };
        let errors = validate(TransactionKind::Withdrawal, &raw).unwrap_err();
        assert_eq!(
            fields(&errors),
            ["instrument_id", "fee", "cash_account_id", "amount"]
        );
    }

    #[test]
    fn income_is_credited_net_of_fee_and_tax() {
        let raw = RawTransaction {
            trade_date: "2026-04-20",
            instrument_id: Some(Uuid::nil()),
            cash_account_id: Some(Uuid::max()),
            amount: Some("1250.00"),
            fee: Some("2.5"),
            tax: Some("125.00"),
            ..Default::default()
        };
        let effects = validate(TransactionKind::Dividend, &raw)
            .unwrap()
            .effects()
            .unwrap();
        assert!(effects.position.is_empty());
        assert_eq!(effects.cash[0].amount_delta, d("1122.50"));

        // Interest is on a cash account and has no instrument.
        let raw = RawTransaction {
            trade_date: "2026-04-30",
            cash_account_id: Some(Uuid::max()),
            amount: Some("3.17"),
            tax: Some("0.47"),
            ..Default::default()
        };
        let effects = validate(TransactionKind::Interest, &raw)
            .unwrap()
            .effects()
            .unwrap();
        assert_eq!(effects.cash[0].amount_delta, d("2.70"));
    }

    #[test]
    fn dividends_may_skip_the_cash_account_but_interest_may_not() {
        let raw = RawTransaction {
            trade_date: "2026-04-20",
            instrument_id: Some(Uuid::nil()),
            amount: Some("10"),
            ..Default::default()
        };
        let dividend = validate(TransactionKind::Dividend, &raw).unwrap();
        assert!(dividend.effects().unwrap().cash.is_empty());

        let errors = validate(TransactionKind::Interest, &raw).unwrap_err();
        assert_eq!(fields(&errors), ["instrument_id", "cash_account_id"]);

        let raw = RawTransaction {
            trade_date: "2026-04-20",
            cash_account_id: Some(Uuid::nil()),
            amount: Some("10"),
            ..Default::default()
        };
        let errors = validate(TransactionKind::Dividend, &raw).unwrap_err();
        assert_eq!(fields(&errors), ["instrument_id"]);
    }

    #[test]
    fn income_charges_may_equal_but_not_exceed_the_gross_amount() {
        let raw = |fee| RawTransaction {
            trade_date: "2026-04-20",
            cash_account_id: Some(Uuid::nil()),
            amount: Some("10.00"),
            fee: Some(fee),
            tax: Some("1"),
            ..Default::default()
        };
        let all_withheld = validate(TransactionKind::Interest, &raw("9.00")).unwrap();
        assert_eq!(all_withheld.effects().unwrap().cash[0].amount_delta, d("0"));

        let errors = validate(TransactionKind::Interest, &raw("9.01")).unwrap_err();
        assert_eq!(errors[0].field, "amount");
        assert_eq!(errors[0].message, "must be at least fee plus tax");
    }

    #[test]
    fn transfers_post_balanced_movements_and_the_source_pays_the_fee() {
        let (from, to) = (Uuid::nil(), Uuid::max());
        let raw = RawTransaction {
            trade_date: "2026-05-01",
            cash_account_id: Some(from),
            destination_cash_account_id: Some(to),
            amount: Some("500.25"),
            ..Default::default()
        };
        let effects = validate(TransactionKind::CashTransfer, &raw)
            .unwrap()
            .effects()
            .unwrap();
        assert_eq!(
            effects.cash,
            [
                CashEffect {
                    cash_account_id: from,
                    amount_delta: d("-500.25"),
                },
                CashEffect {
                    cash_account_id: to,
                    amount_delta: d("500.25"),
                },
            ]
        );
        let sum: Decimal = effects.cash.iter().map(|c| c.amount_delta).sum();
        assert!(sum.is_zero());

        let with_fee = RawTransaction {
            fee: Some("1.50"),
            ..raw
        };
        let effects = validate(TransactionKind::CashTransfer, &with_fee)
            .unwrap()
            .effects()
            .unwrap();
        assert_eq!(effects.cash[0].amount_delta, d("-501.75"));
        assert_eq!(effects.cash[1].amount_delta, d("500.25"));
    }

    #[test]
    fn transfers_need_two_different_accounts() {
        let raw = RawTransaction {
            trade_date: "2026-05-01",
            cash_account_id: Some(Uuid::nil()),
            destination_cash_account_id: Some(Uuid::nil()),
            amount: Some("1"),
            ..Default::default()
        };
        let errors = validate(TransactionKind::CashTransfer, &raw).unwrap_err();
        assert_eq!(fields(&errors), ["destination_cash_account_id"]);

        let raw = RawTransaction {
            trade_date: "2026-05-01",
            cash_account_id: Some(Uuid::nil()),
            amount: Some("1"),
            tax: Some("1"),
            ..Default::default()
        };
        let errors = validate(TransactionKind::CashTransfer, &raw).unwrap_err();
        assert_eq!(fields(&errors), ["tax", "destination_cash_account_id"]);

        // Other kinds do not take a destination.
        let raw = RawTransaction {
            trade_date: "2026-05-01",
            cash_account_id: Some(Uuid::nil()),
            destination_cash_account_id: Some(Uuid::max()),
            amount: Some("1"),
            ..Default::default()
        };
        let errors = validate(TransactionKind::Deposit, &raw).unwrap_err();
        assert_eq!(fields(&errors), ["destination_cash_account_id"]);
    }

    #[test]
    fn fx_exchanges_spend_amount_plus_fee_and_receive_the_destination_amount() {
        // Snowball's example: buy 82 EUR for 100 USD plus a 2.5 USD fee.
        let (usd, eur) = (Uuid::nil(), Uuid::max());
        let raw = RawTransaction {
            trade_date: "2020-01-08",
            cash_account_id: Some(usd),
            destination_cash_account_id: Some(eur),
            amount: Some("100"),
            destination_amount: Some("82"),
            fee: Some("2.5"),
            ..Default::default()
        };
        let effects = validate(TransactionKind::FxExchange, &raw)
            .unwrap()
            .effects()
            .unwrap();
        assert!(effects.position.is_empty());
        assert_eq!(
            effects.cash,
            [
                CashEffect {
                    cash_account_id: usd,
                    amount_delta: d("-102.5"),
                },
                CashEffect {
                    cash_account_id: eur,
                    amount_delta: d("82"),
                },
            ]
        );

        let missing = RawTransaction {
            destination_amount: None,
            ..raw
        };
        let errors = validate(TransactionKind::FxExchange, &missing).unwrap_err();
        assert_eq!(fields(&errors), ["destination_amount"]);
        // A transfer has no destination amount.
        let errors = validate(TransactionKind::CashTransfer, &raw).unwrap_err();
        assert_eq!(fields(&errors), ["destination_amount"]);
    }

    #[test]
    fn splits_post_their_ratio_and_optional_cash_in_lieu() {
        let raw = RawTransaction {
            trade_date: "2026-06-01",
            instrument_id: Some(Uuid::nil()),
            ratio: Some("1:3"),
            ..Default::default()
        };
        let effects = validate(TransactionKind::StockSplit, &raw)
            .unwrap()
            .effects()
            .unwrap();
        assert_eq!(
            effects.position,
            [PositionEffect {
                instrument_id: Uuid::nil(),
                kind: MovementKind::Split,
                quantity: d("1"),
                amount: d("3"),
            }]
        );
        assert!(effects.cash.is_empty());

        let with_cash = RawTransaction {
            cash_in_lieu_quantity: Some("0.33333333"),
            amount: Some("4.10"),
            cash_account_id: Some(Uuid::max()),
            ..raw
        };
        let effects = validate(TransactionKind::StockSplit, &with_cash)
            .unwrap()
            .effects()
            .unwrap();
        assert_eq!(effects.position[1].kind, MovementKind::Dispose);
        assert_eq!(effects.position[1].quantity, d("0.33333333"));
        assert_eq!(effects.position[1].amount, d("4.10"));
        assert_eq!(effects.cash[0].amount_delta, d("4.10"));
    }

    #[test]
    fn stock_dividends_post_the_new_shares_at_no_cost() {
        let raw = RawTransaction {
            trade_date: "2026-06-01",
            instrument_id: Some(Uuid::nil()),
            quantity: Some("4"),
            ..Default::default()
        };
        let effects = validate(TransactionKind::StockDividend, &raw)
            .unwrap()
            .effects()
            .unwrap();
        assert_eq!(effects.position[0].kind, MovementKind::StockDividend);
        assert_eq!(effects.position[0].quantity, d("4"));
        assert!(effects.position[0].amount.is_zero());
    }

    #[test]
    fn cash_in_lieu_needs_both_quantity_and_amount() {
        let base = RawTransaction {
            trade_date: "2026-06-01",
            instrument_id: Some(Uuid::nil()),
            ratio: Some("3:2"),
            ..Default::default()
        };
        for (raw, expected) in [
            (
                RawTransaction {
                    cash_in_lieu_quantity: Some("0.5"),
                    ..base.clone()
                },
                "amount",
            ),
            (
                RawTransaction {
                    amount: Some("5"),
                    ..base.clone()
                },
                "cash_in_lieu_quantity",
            ),
            (
                RawTransaction {
                    cash_account_id: Some(Uuid::max()),
                    ..base.clone()
                },
                "cash_account_id",
            ),
            (
                RawTransaction {
                    ratio: Some("2:2"),
                    ..base.clone()
                },
                "ratio",
            ),
        ] {
            let errors = validate(TransactionKind::StockSplit, &raw).unwrap_err();
            assert_eq!(fields(&errors), [expected]);
        }
        let errors = validate(
            TransactionKind::StockSplit,
            &RawTransaction {
                ratio: None,
                ..base
            },
        )
        .unwrap_err();
        assert_eq!(fields(&errors), ["ratio"]);
    }

    #[test]
    fn kinds_round_trip_through_their_stored_names() {
        for kind in [
            TransactionKind::OpeningPosition,
            TransactionKind::OpeningCash,
            TransactionKind::Buy,
            TransactionKind::Sell,
            TransactionKind::Deposit,
            TransactionKind::Withdrawal,
            TransactionKind::Dividend,
            TransactionKind::Interest,
            TransactionKind::Fee,
            TransactionKind::Tax,
            TransactionKind::TaxReturn,
            TransactionKind::CashTransfer,
            TransactionKind::FxExchange,
            TransactionKind::StockSplit,
            TransactionKind::StockDividend,
        ] {
            assert_eq!(TransactionKind::from_db(kind.as_str()).unwrap(), kind);
        }
        assert!(TransactionKind::from_db("SPIN_OFF").is_err());
        for kind in [
            MovementKind::Acquire,
            MovementKind::Dispose,
            MovementKind::Split,
            MovementKind::StockDividend,
        ] {
            assert_eq!(MovementKind::from_db(kind.as_str()), Some(kind));
        }
    }

    #[test]
    fn dates_are_strict_and_bounded() {
        assert!(parse_date("2026-01-31").is_ok());
        for bad in [
            "2026-1-31",
            "31/01/2026",
            "2026-02-29",
            "1899-12-31",
            "2101-01-01",
            "",
        ] {
            assert!(parse_date(bad).is_err(), "{bad}");
        }
    }
}
