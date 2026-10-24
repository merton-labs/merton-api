//! Exact decimal values for the ledger (ADR-NUM-01 … NUM-04).
//!
//! Values are parsed from strings with a strict grammar, kept at the scale
//! they were entered with, and stored as canonical decimal text. Money,
//! quantity and price are distinct types so they cannot be mixed up.

use std::fmt;
use std::str::FromStr;

use rust_decimal::Decimal;

/// Maximum digits after the decimal point for any ledger value.
pub const MAX_SCALE: u32 = 8;
/// Maximum digits before the decimal point for any ledger value.
pub const MAX_INTEGER_DIGITS: usize = 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sign {
    Positive,
    NonNegative,
    Any,
}

/// Parses `input` as a plain decimal: optional `-`, digits, optional
/// fraction. Exponents, `+`, `_` separators and whitespace inside the
/// number are rejected; surrounding whitespace is ignored.
fn parse(input: &str, sign: Sign) -> Result<Decimal, String> {
    let text = input.trim();
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    let (integer, fraction) = match unsigned.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (unsigned, None),
    };
    let digits_only = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits_only(integer) || fraction.is_some_and(|f| !digits_only(f)) {
        return Err("must be a decimal number such as 12.50".to_owned());
    }
    if integer.trim_start_matches('0').len() > MAX_INTEGER_DIGITS {
        return Err(format!(
            "must have at most {MAX_INTEGER_DIGITS} digits before the decimal point"
        ));
    }
    if fraction.is_some_and(|f| f.len() > MAX_SCALE as usize) {
        return Err(format!(
            "must have at most {MAX_SCALE} digits after the decimal point"
        ));
    }

    let mut value = Decimal::from_str_exact(text)
        .map_err(|_| "must be a decimal number such as 12.50".to_owned())?;
    if value.is_zero() {
        value.set_sign_positive(true);
    }
    match sign {
        Sign::Positive if value <= Decimal::ZERO => Err("must be greater than zero".to_owned()),
        Sign::NonNegative if value.is_sign_negative() => Err("must not be negative".to_owned()),
        _ => Ok(value),
    }
}

// `Decimal` arithmetic rounds silently when an exact result needs more
// than 28 significant digits, and its result scale is not a reliable
// signal (it drops the scale of zero results). These helpers work on the
// integer mantissa instead, so they either return the exact value or
// `None`.

/// `a × b` without rounding; `None` if the exact product is not
/// representable.
pub fn exact_mul(a: Decimal, b: Decimal) -> Option<Decimal> {
    let mantissa = a.mantissa().checked_mul(b.mantissa())?;
    from_parts(mantissa, a.scale() + b.scale())
}

/// `a + b` without rounding; `None` if the exact sum is not representable.
pub fn exact_add(a: Decimal, b: Decimal) -> Option<Decimal> {
    let scale = a.scale().max(b.scale());
    from_parts(rescaled(a, scale)?.checked_add(rescaled(b, scale)?)?, scale)
}

/// `a − b` without rounding; `None` if the exact difference is not
/// representable.
pub fn exact_sub(a: Decimal, b: Decimal) -> Option<Decimal> {
    let scale = a.scale().max(b.scale());
    from_parts(rescaled(a, scale)?.checked_sub(rescaled(b, scale)?)?, scale)
}

/// The mantissa of `value` expressed at `scale` (≥ its own scale).
fn rescaled(value: Decimal, scale: u32) -> Option<i128> {
    let factor = 10i128.checked_pow(scale - value.scale())?;
    value.mantissa().checked_mul(factor)
}

fn from_parts(mantissa: i128, scale: u32) -> Option<Decimal> {
    Decimal::try_from_i128_with_scale(mantissa, scale).ok()
}

fn sign_allows(sign: Sign, value: Decimal) -> bool {
    match sign {
        Sign::Positive => value > Decimal::ZERO,
        Sign::NonNegative => !value.is_sign_negative() || value.is_zero(),
        Sign::Any => true,
    }
}

macro_rules! ledger_decimal {
    ($(#[$doc:meta])* $name:ident, $sign:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
        pub struct $name(Decimal);

        impl $name {
            pub fn parse(input: &str) -> Result<Self, String> {
                parse(input, $sign).map(Self)
            }

            /// Reads a value written by this application. The sign rule is
            /// re-checked; the input scale limit is not, because derived
            /// values (such as quantity × price) can carry more decimals.
            pub fn from_stored(text: &str) -> Result<Self, String> {
                let value = Decimal::from_str(text)
                    .map_err(|err| format!("invalid stored decimal {text:?}: {err}"))?;
                if sign_allows($sign, value) {
                    Ok(Self(value))
                } else {
                    Err(format!("stored decimal {text:?} violates its sign rule"))
                }
            }

            pub fn value(self) -> Decimal {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

ledger_decimal!(
    /// A number of units held or traded; always greater than zero.
    Quantity,
    Sign::Positive
);
ledger_decimal!(
    /// A per-unit price in the instrument's quote currency; zero or more.
    Price,
    Sign::NonNegative
);
ledger_decimal!(
    /// A non-negative amount of money, such as a fee, tax or cost basis.
    Amount,
    Sign::NonNegative
);
ledger_decimal!(
    /// A signed amount of money, such as an opening cash balance or a cash
    /// movement.
    SignedAmount,
    Sign::Any
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_entered_scale() {
        assert_eq!(Amount::parse("100.50").unwrap().to_string(), "100.50");
        assert_eq!(Amount::parse(" 7 ").unwrap().to_string(), "7");
        assert_eq!(
            Quantity::parse("0.00000001").unwrap().to_string(),
            "0.00000001"
        );
        assert_eq!(SignedAmount::parse("-12.30").unwrap().to_string(), "-12.30");
        assert_eq!(Price::parse("007.5").unwrap().to_string(), "7.5");
    }

    #[test]
    fn enforces_limits() {
        assert!(Amount::parse(&format!("{}.12", "9".repeat(15))).is_ok());
        assert!(Amount::parse(&"9".repeat(16)).is_err());
        assert!(Amount::parse("1.123456789").is_err());
        assert!(Amount::parse(&format!("{}1", "0".repeat(20))).is_ok());
    }

    #[test]
    fn rejects_non_plain_decimals() {
        for bad in [
            "", "-", ".5", "5.", "1e3", "+1", "1_000", "1,000", "1 000", "NaN", "--1", "0x10",
        ] {
            assert!(SignedAmount::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn enforces_sign_rules() {
        assert!(Quantity::parse("0").is_err());
        assert!(Quantity::parse("-1").is_err());
        assert!(Price::parse("0").is_ok());
        assert!(Price::parse("-0.01").is_err());
        assert!(Amount::parse("-1").is_err());
        assert_eq!(Amount::parse("-0.00").unwrap().to_string(), "0.00");
        assert!(SignedAmount::parse("-1").is_ok());
    }

    #[test]
    fn exact_arithmetic_refuses_to_round() {
        let d = |s: &str| s.parse::<Decimal>().unwrap();
        assert_eq!(exact_mul(d("1.50"), d("2.0")), Some(d("3.000")));
        // 30 significant digits: Decimal would round this silently.
        assert_eq!(
            exact_mul(d("10000000.12345678"), d("1000000.12345678")),
            None
        );
        assert_eq!(exact_add(d("1.10"), d("2.2")), Some(d("3.30")));
        assert_eq!(
            exact_add(d("99999999999999.99999999999999"), d("0.00000000000001")),
            Some(d("100000000000000.00000000000000"))
        );
        assert_eq!(
            exact_add(d("9999999999999999999999.999999"), d("0.000001")),
            Some(d("10000000000000000000000.000000"))
        );
        assert_eq!(
            exact_add(d("99999999999999999999999.99999"), d("0.000001")),
            None
        );
        assert_eq!(exact_sub(d("5"), d("0.25")), Some(d("4.75")));
        let max = Decimal::MAX;
        assert_eq!(exact_add(max, d("1")), None);
        assert_eq!(exact_sub(Decimal::MIN, d("1")), None);
    }

    #[test]
    fn exact_arithmetic_handles_zero() {
        let d = |s: &str| s.parse::<Decimal>().unwrap();
        assert_eq!(exact_add(d("0.00"), d("0")), Some(d("0.00")));
        assert_eq!(exact_sub(d("0.00"), d("0")), Some(d("0")));
        assert_eq!(exact_sub(d("1.5"), d("0.00")), Some(d("1.50")));
        assert_eq!(exact_mul(d("0.00"), d("20")), Some(d("0")));
        assert_eq!(exact_mul(d("1.00"), d("0")), Some(d("0")));
    }

    #[test]
    fn stored_values_are_rechecked() {
        assert!(Quantity::from_stored("-5").is_err());
        assert!(Quantity::from_stored("0").is_err());
        assert!(Amount::from_stored("-0.01").is_err());
        assert!(SignedAmount::from_stored("-0.01").is_ok());
        assert!(Quantity::from_stored("1.0000000000000001").is_ok());
        assert!(Quantity::from_stored("abc").is_err());
    }

    #[test]
    fn stored_text_round_trips() {
        let value = Amount::parse("1234.5600").unwrap();
        assert_eq!(Amount::from_stored(&value.to_string()).unwrap(), value);
        assert_eq!(
            Amount::from_stored(&value.to_string()).unwrap().to_string(),
            "1234.5600"
        );
    }
}
