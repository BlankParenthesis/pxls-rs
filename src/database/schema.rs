table! {
    board (id) {
        id -> Int4,
        name -> Text,
        created_at -> Int8,
        shape -> Text,
        mask -> Bytea,
        initial -> Bytea,
    }
}

table! {
    color (board, index) {
        board -> Int4,
        index -> Int4,
        name -> Text,
        value -> Int4,
    }
}

table! {
    placement (id) {
        id -> Int8,
        board -> Int4,
        position -> Int8,
        color -> Int2,
        timestamp -> Int4,
        user_id -> Nullable<Text>,
    }
}

joinable!(color -> board (board));
joinable!(placement -> board (board));

allow_tables_to_appear_in_same_query!(
    board,
    color,
    placement,
);
