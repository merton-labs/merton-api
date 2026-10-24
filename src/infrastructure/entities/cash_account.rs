use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "cash_accounts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub portfolio_id: Uuid,
    pub name: String,
    pub currency: String,
    pub created_at: TimeDateTimeWithTimeZone,
    pub updated_at: TimeDateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::portfolio::Entity",
        from = "Column::PortfolioId",
        to = "super::portfolio::Column::Id",
        on_delete = "Restrict"
    )]
    Portfolio,
}

impl Related<super::portfolio::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Portfolio.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
