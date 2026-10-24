use std::fmt;

use sea_orm::entity::prelude::*;

#[derive(Clone, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "users")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub email: String,
    #[sea_orm(unique)]
    pub normalized_email: String,
    pub password_hash: String,
    pub created_at: TimeDateTimeWithTimeZone,
    pub updated_at: TimeDateTimeWithTimeZone,
}

// Hand-written so the password hash never reaches logs.
impl fmt::Debug for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("user::Model")
            .field("id", &self.id)
            .field("email", &self.email)
            .field("password_hash", &"<redacted>")
            .finish_non_exhaustive()
    }
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::user_session::Entity")]
    UserSession,
}

impl Related<super::user_session::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::UserSession.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
