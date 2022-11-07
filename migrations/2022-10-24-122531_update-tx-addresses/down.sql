-- This file should undo anything in `up.sql`

DROP TABLE transaction_addresses;

CREATE TABLE transaction_addresses (
    hash varchar not null,
	txid varchar not null,
	address varchar not null,
	PRIMARY KEY(hash, txid, address)
);

ALTER TABLE chaintips
DROP COLUMN parent_block;
