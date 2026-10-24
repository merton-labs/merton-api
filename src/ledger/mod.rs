//! The transaction ledger: the authoritative history of a portfolio
//! (ADR-LEDG-01). Holdings and cash balances are derived from movements and
//! never stored.

pub mod decimal;
pub mod fifo;
pub(crate) mod repository;
pub mod service;
pub mod transaction;
