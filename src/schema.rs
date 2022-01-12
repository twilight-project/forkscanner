table! {
    blocks (hash) {
        hash -> Varchar,
        height -> Int8,
        parent_hash -> Nullable<Varchar>,
        connected -> Bool,
        first_seen_by -> Int8,
        headers_only -> Bool,
    }
}

table! {
    chaintips (id) {
        id -> Int8,
        node -> Int8,
        status -> Varchar,
        block -> Varchar,
        height -> Int8,
        parent_chaintip -> Nullable<Int8>,
    }
}

table! {
    invalid_blocks (hash, node) {
        hash -> Varchar,
        node -> Int8,
    }
}

table! {
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

table! {
    peers (id) {
        id -> Int8,
        node_id -> Int8,
        peer_id -> Int8,
        address -> Varchar,
        version -> Int8,
    }
}

table! {
    valid_blocks (hash, node) {
        hash -> Varchar,
        node -> Int8,
    }
}

joinable!(peers -> nodes (node_id));

allow_tables_to_appear_in_same_query!(
    blocks,
    chaintips,
    invalid_blocks,
    nodes,
    peers,
    valid_blocks,
);
