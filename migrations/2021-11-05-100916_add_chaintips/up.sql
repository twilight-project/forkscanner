-- Your SQL goes here
CREATE TABLE chaintips (
	id bigint generated always as identity,
	node varchar not null,
	status varchar not null,
	block varchar not null,
	height bigint not null,
	parent_chaintip bigint,
	PRIMARY KEY(id),
	CONSTRAINT fk_parent
	  FOREIGN KEY(parent_chaintip)
	    REFERENCES chaintips(id)
	    ON DELETE SET NULL
);

ALTER TABLE blocks ADD COLUMN marked_valid varchar;
ALTER TABLE blocks ADD COLUMN marked_invalid varchar;
