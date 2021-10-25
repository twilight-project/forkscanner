table! {
    blocks (hash) {
        hash -> Varchar,
        height -> Int8,
        parent_hash -> Nullable<Varchar>,
        connected -> Bool,
    }
}

table! {
    chaintips (id) {
        id -> Int8,
        height -> Int8,
        hash -> Varchar,
        branchlen -> Int4,
        status -> Varchar,
    }
}

allow_tables_to_appear_in_same_query!(
    blocks,
    chaintips,
);
