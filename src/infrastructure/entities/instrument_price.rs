use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "instrument_prices")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub instrument_id: Uuid,
    pub price_date: TimeDate,
    /// Decimal text in the instrument's quote currency (ADR-NUM-02).
    pub price: String,
    /// Where the price came from; see `crate::market_data::Source`.
    pub source: String,
    pub created_at: TimeDateTimeWithTimeZone,
    pub updated_at: TimeDateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::instrument::Entity",
        from = "Column::InstrumentId",
        to = "super::instrument::Column::Id",
        on_delete = "Restrict"
    )]
    Instrument,
}

impl Related<super::instrument::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Instrument.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
