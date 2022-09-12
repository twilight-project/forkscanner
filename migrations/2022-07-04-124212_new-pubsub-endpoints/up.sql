-- Your SQL goes here
CREATE TABLE watched (
    address varchar not null,
	created_at timestamp with time zone not null,
	watch_until timestamp with time zone not null,
	PRIMARY KEY(address)
);

CREATE TABLE transaction_addresses (
    hash varchar not null,
	txid varchar not null,
	address varchar not null,
	PRIMARY KEY(hash, txid, address)
);

CREATE TABLE lags (
    node_id bigint not null,
	created_at timestamp with time zone not null,
	deleted_at timestamp with time zone,
	updated_at timestamp with time zone not null,
	PRIMARY KEY(node_id, created_at, updated_at),
	CONSTRAINT fk_lags_node_id
	    FOREIGN KEY(node_id)
		    REFERENCES nodes(id)
			ON DELETE CASCADE
);

ALTER TABLE invalid_blocks
ADD column created_at timestamp with time zone;

ALTER TABLE valid_blocks
ADD column created_at timestamp with time zone;

ALTER TABLE transaction
ADD COLUMN swept bool;

ALTER TABLE nodes
ADD COLUMN mirror_host varchar DEFAULT NULL, 
ADD COLUMN mirror_last_polled timestamp with time zone DEFAULT NULL,
ADD COLUMN mirror_unreachable_since bigint DEFAULT NULL,
ADD COLUMN archive boolean NOT NULL DEFAULT FALSE;
