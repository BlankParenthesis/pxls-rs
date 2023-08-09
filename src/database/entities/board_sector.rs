//! `SeaORM` Entity. Generated by sea-orm-codegen 0.11.2



use sea_orm :: entity :: prelude :: * ; use serde :: { Deserialize , Serialize } ;

# [derive (Clone , Debug , PartialEq , DeriveEntityModel , Eq , Serialize , Deserialize)] # [sea_orm (table_name = "board_sector")] pub struct Model { # [sea_orm (primary_key , auto_increment = false)] # [serde (skip_deserializing)] pub board : i32 , # [sea_orm (primary_key , auto_increment = false)] # [serde (skip_deserializing)] pub sector : i32 , # [sea_orm (column_type = "Binary(BlobSize::Blob(None))")] pub mask : Vec < u8 > , # [sea_orm (column_type = "Binary(BlobSize::Blob(None))")] pub initial : Vec < u8 > , }

# [derive (Copy , Clone , Debug , EnumIter , DeriveRelation)] pub enum Relation { # [sea_orm (belongs_to = "super::board::Entity" , from = "Column::Board" , to = "super::board::Column::Id" , on_update = "NoAction" , on_delete = "NoAction" ,)] Board , }

impl Related < super :: board :: Entity > for Entity { fn to () -> RelationDef { Relation :: Board . def () } }

impl ActiveModelBehavior for ActiveModel { }