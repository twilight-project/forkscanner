-- Your SQL goes here

DROP TABLE transaction_addresses;

CREATE TABLE transaction_addresses (
    created_at timestamptz not null,
	txid varchar not null,
	address varchar not null,
	direction varchar not null,
	PRIMARY KEY(txid, address)
);

ALTER TABLE chaintips
ADD COLUMN parent_block varchar;
