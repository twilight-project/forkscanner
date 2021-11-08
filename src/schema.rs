table! {
    blocks (hash) {
        hash -> Varchar,
        height -> Int8,
        parent_hash -> Nullable<Varchar>,
        connected -> Bool,
        marked_valid -> Nullable<Varchar>,
        marked_invalid -> Nullable<Varchar>,
    }
}

table! {
    chaintips (id) {
        id -> Int8,
        node -> Varchar,
        status -> Varchar,
        block -> Varchar,
        height -> Int8,
        parent_chaintip -> Nullable<Int8>,
    }
}

allow_tables_to_appear_in_same_query!(
    blocks,
    chaintips,
);
