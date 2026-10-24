use sea_orm::entity::prelude::*;

/// A ledger transaction as entered. Decimal columns hold canonical decimal
/// text; see `crate::ledger::decimal`.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "transactions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub portfolio_id: Uuid,
    pub kind: String,
    pub trade_date: TimeDate,
    pub sort_order: i32,
    pub instrument_id: Option<Uuid>,
    pub cash_account_id: Option<Uuid>,
    pub quantity: Option<String>,
    pub price: Option<String>,
    pub fee: Option<String>,
    pub tax: Option<String>,
    pub amount: Option<String>,
    pub note: Option<String>,
    pub created_at: TimeDateTimeWithTimeZone,
    pub updated_at: TimeDateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::position_movement::Entity")]
    PositionMovement,
    #[sea_orm(has_many = "super::cash_movement::Entity")]
    CashMovement,
}

impl Related<super::position_movement::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::PositionMovement.def()
    }
}

impl Related<super::cash_movement::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::CashMovement.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
