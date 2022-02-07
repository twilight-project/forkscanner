// @generated automatically by Diesel CLI.

diesel::table! {
    blocks (hash) {
        hash -> Varchar,
        height -> Int8,
        parent_hash -> Nullable<Varchar>,
        connected -> Bool,
        first_seen_by -> Int8,
        headers_only -> Bool,
        work -> Varchar,
    }
}

diesel::table! {
    chaintips (id) {
        id -> Int8,
        node -> Int8,
        status -> Varchar,
        block -> Varchar,
        height -> Int8,
        parent_chaintip -> Nullable<Int8>,
    }
}

diesel::table! {
    double_spent_by (candidate_height, txid) {
        candidate_height -> Int8,
        txid -> Varchar,
    }
}

diesel::table! {
    invalid_blocks (hash, node) {
        hash -> Varchar,
        node -> Int8,
    }
}

diesel::table! {
    nodes (id) {
        id -> Int8,
        node -> Varchar,
        rpc_host -> Varchar,
        rpc_port -> Int4,
        mirror_rpc_port -> Nullable<Int4>,
        rpc_user -> Varchar,
        rpc_pass -> Varchar,
        unreachable_since -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    peers (id) {
        id -> Int8,
        node_id -> Int8,
        peer_id -> Int8,
        address -> Varchar,
        version -> Int8,
    }
}

diesel::table! {
    rbf_by (candidate_height, txid) {
        candidate_height -> Int8,
        txid -> Varchar,
    }
}

diesel::table! {
    stale_candidate (height) {
        height -> Int8,
        n_children -> Int4,
        confirmed_in_one_branch_total -> Float8,
        double_spent_in_one_branch_total -> Float8,
        rbf_total -> Float8,
        height_processed -> Nullable<Int8>,
    }
}

diesel::table! {
    stale_candidate_children (root_id) {
        candidate_height -> Int8,
        root_id -> Varchar,
        tip_id -> Varchar,
        len -> Int4,
    }
}

diesel::table! {
    transaction (block_id, txid) {
        block_id -> Varchar,
        txid -> Varchar,
        is_coinbase -> Bool,
        hex -> Varchar,
        amount -> Float8,
    }
}

diesel::table! {
    valid_blocks (hash, node) {
        hash -> Varchar,
        node -> Int8,
    }
}

diesel::joinable!(peers -> nodes (node_id));
diesel::joinable!(stale_candidate_children -> stale_candidate (candidate_height));

diesel::allow_tables_to_appear_in_same_query!(
    blocks,
    chaintips,
    double_spent_by,
    invalid_blocks,
    nodes,
    peers,
    rbf_by,
    stale_candidate,
    stale_candidate_children,
    transaction,
    valid_blocks,
);
