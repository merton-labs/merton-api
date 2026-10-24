//! Transaction kinds, input validation and their effects on positions and
//! cash (ADR-LEDG-04 … LEDG-07, ADR-TXN-02).

use rust_decimal::Decimal;
use sea_orm::DbErr;
use time::Date;
use time::macros::{date, format_description};
use uuid::Uuid;

use super::decimal::{Amount, Price, Quantity, SignedAmount, exact_add, exact_mul, exact_sub};
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
}

impl TransactionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpeningPosition => "OPENING_POSITION",
            Self::OpeningCash => "OPENING_CASH",
            Self::Buy => "BUY",
            Self::Sell => "SELL",
        }
    }

    pub fn from_db(value: &str) -> Result<Self, DbErr> {
        match value {
            "OPENING_POSITION" => Ok(Self::OpeningPosition),
            "OPENING_CASH" => Ok(Self::OpeningCash),
            "BUY" => Ok(Self::Buy),
            "SELL" => Ok(Self::Sell),
            other => Err(DbErr::Type(format!("unknown transaction kind {other:?}"))),
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
    pub quantity: Option<&'a str>,
    pub price: Option<&'a str>,
    pub fee: Option<&'a str>,
    pub tax: Option<&'a str>,
    pub amount: Option<&'a str>,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidTransaction {
    pub kind: TransactionKind,
    pub trade_date: Date,
    pub sort_order: i32,
    pub details: Details,
    pub note: Option<String>,
}

/// The effect on one instrument: signed quantity, and the lot cost (for an
/// acquisition) or net proceeds (for a disposal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PositionEffect {
    pub instrument_id: Uuid,
    pub quantity_delta: Decimal,
    pub amount: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CashEffect {
    pub cash_account_id: Uuid,
    pub amount_delta: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Effects {
    pub position: Option<PositionEffect>,
    pub cash: Option<CashEffect>,
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

    let details = match kind {
        TransactionKind::OpeningPosition => {
            v.forbid("cash_account_id", raw.cash_account_id.is_some(), kind);
            v.forbid("price", raw.price.is_some(), kind);
            v.forbid("fee", raw.fee.is_some(), kind);
            v.forbid("tax", raw.tax.is_some(), kind);
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
            v.forbid("instrument_id", raw.instrument_id.is_some(), kind);
            v.forbid("quantity", raw.quantity.is_some(), kind);
            v.forbid("price", raw.price.is_some(), kind);
            v.forbid("fee", raw.fee.is_some(), kind);
            v.forbid("tax", raw.tax.is_some(), kind);
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
            v.forbid("amount", raw.amount.is_some(), kind);
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
            // Reject values whose derived amounts overflow up front.
            valid.effects().map_err(|message| {
                vec![FieldError {
                    field: "quantity",
                    message,
                }]
            })?;
            Ok(valid)
        }
        _ => Err(v.errors),
    }
}

impl ValidTransaction {
    pub fn instrument_id(&self) -> Option<Uuid> {
        match self.details {
            Details::OpeningPosition { instrument_id, .. }
            | Details::Trade { instrument_id, .. } => Some(instrument_id),
            Details::OpeningCash { .. } => None,
        }
    }

    pub fn cash_account_id(&self) -> Option<Uuid> {
        match self.details {
            Details::OpeningCash {
                cash_account_id, ..
            } => Some(cash_account_id),
            Details::Trade {
                cash_account_id, ..
            } => cash_account_id,
            Details::OpeningPosition { .. } => None,
        }
    }

    /// The movements this transaction posts. All arithmetic is exact:
    /// a result that would overflow or need rounding is rejected.
    ///
    /// - Buy: cost = quantity × price + fee + tax; cash −cost.
    /// - Sell: proceeds = quantity × price − fee − tax; cash +proceeds.
    pub fn effects(&self) -> Result<Effects, String> {
        let overflow = || "the resulting amount is too large or too precise to be exact".to_owned();
        Ok(match self.details {
            Details::OpeningPosition {
                instrument_id,
                quantity,
                cost_basis,
            } => Effects {
                position: Some(PositionEffect {
                    instrument_id,
                    quantity_delta: quantity.value(),
                    amount: cost_basis.value(),
                }),
                cash: None,
            },
            Details::OpeningCash {
                cash_account_id,
                amount,
            } => Effects {
                position: None,
                cash: Some(CashEffect {
                    cash_account_id,
                    amount_delta: amount.value(),
                }),
            },
            Details::Trade {
                instrument_id,
                cash_account_id,
                quantity,
                price,
                fee,
                tax,
            } => {
                let gross = exact_mul(quantity.value(), price.value()).ok_or_else(overflow)?;
                let charges = exact_add(fee.value(), tax.value()).ok_or_else(overflow)?;
                let (quantity_delta, amount, cash_delta) = if self.kind == TransactionKind::Buy {
                    let cost = exact_add(gross, charges).ok_or_else(overflow)?;
                    (quantity.value(), cost, -cost)
                } else {
                    let proceeds = exact_sub(gross, charges).ok_or_else(overflow)?;
                    (-quantity.value(), proceeds, proceeds)
                };
                Effects {
                    position: Some(PositionEffect {
                        instrument_id,
                        quantity_delta,
                        amount,
                    }),
                    cash: cash_account_id.map(|cash_account_id| CashEffect {
                        cash_account_id,
                        amount_delta: cash_delta,
                    }),
                }
            }
        })
    }
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

    #[test]
    fn buy_cost_includes_fees_and_taxes() {
        let effects = trade(TransactionKind::Buy, "100", "35.25", "15.50", "1.09");

        let position = effects.position.unwrap();
        assert_eq!(position.quantity_delta, d("100"));
        assert_eq!(position.amount, d("3541.59")); // 100 × 35.25 = 3525; + 15.50 + 1.09
        assert_eq!(effects.cash.unwrap().amount_delta, d("-3541.59"));
    }

    #[test]
    fn sell_proceeds_are_net_of_fees_and_taxes() {
        let effects = trade(TransactionKind::Sell, "40", "50", "12", "0.84");

        let position = effects.position.unwrap();
        assert_eq!(position.quantity_delta, d("-40"));
        assert_eq!(position.amount, d("1987.16"));
        assert_eq!(effects.cash.unwrap().amount_delta, d("1987.16"));
    }

    #[test]
    fn products_are_exact() {
        let effects = trade(TransactionKind::Buy, "0.33333333", "3.00000001", "0", "0");

        // 0.33333333 × 3.00000001 = 0.99999999 + 0.0000000033333333
        assert_eq!(effects.position.unwrap().amount, d("0.9999999933333333"));
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
        let fields: Vec<_> = errors.iter().map(|e| e.field).collect();
        assert_eq!(
            fields,
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
        assert_eq!(
            cash.effects().unwrap().cash.unwrap().amount_delta,
            d("-250.00")
        );

        let raw = RawTransaction {
            trade_date: "2026-01-01",
            instrument_id: Some(Uuid::nil()),
            quantity: Some("1"),
            price: Some("10"),
            ..Default::default()
        };
        let buy = validate(TransactionKind::Buy, &raw).unwrap();
        assert!(buy.effects().unwrap().cash.is_none());
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
