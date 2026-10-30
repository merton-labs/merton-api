use sea_orm::entity::prelude::*;

/// One transaction's effect on one instrument: `ACQUIRE` opens a lot with
/// `amount` as its cost; `DISPOSE` consumes lots for `amount` net proceeds;
/// `SPLIT` multiplies the holding by `quantity / amount` (new : old shares);
/// `STOCK_DIVIDEND` adds `quantity` shares over the open lots (`amount` 0).
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "position_movements")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub transaction_id: Uuid,
    pub portfolio_id: Uuid,
    pub instrument_id: Uuid,
    pub kind: String,
    pub quantity: String,
    pub amount: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::transaction::Entity",
        from = "Column::TransactionId",
        to = "super::transaction::Column::Id",
        on_delete = "Restrict"
    )]
    Transaction,
}

impl Related<super::transaction::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Transaction.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
