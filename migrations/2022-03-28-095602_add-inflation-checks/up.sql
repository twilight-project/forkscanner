-- Your SQL goes here
CREATE TABLE inflated_blocks (
    block_hash varchar primary key,
	max_inflation decimal not null,
	actual_inflation decimal not null,
	notified_at timestamp with time zone not null,
	created_at timestamp with time zone not null,
	updated_at timestamp with time zone not null,
	node_id bigint not null,
	dismissed_at timestamp with time zone ,
	CONSTRAINT fk_inflated_blocks_hash
	  FOREIGN KEY(block_hash)
	    REFERENCES blocks(hash)
		ON DELETE CASCADE
);

CREATE TABLE tx_outsets (
    block_hash varchar not null,
	node_id bigint not null,
	txouts bigint not null,
	total_amount decimal not null,
	created_at timestamp with time zone not null,
	updated_at timestamp with time zone not null,
	inflated boolean not null,
	PRIMARY KEY (block_hash, node_id),
	CONSTRAINT fk_tx_outsets_block_hash
	  FOREIGN KEY(block_hash)
	    REFERENCES blocks(hash)
);

ALTER TABLE nodes
ADD COLUMN last_polled timestamp with time zone;

ALTER TABLE nodes
ADD COLUMN initial_block_download boolean not null default true;
