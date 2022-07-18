-- Your SQL goes here
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
