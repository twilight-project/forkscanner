-- This file should undo anything in `up.sql`
DROP TABLE stale_candidate_children;
DROP TABLE stale_candidate;
DROP TABLE transaction;

ALTER TABLE blocks
DROP COLUMN work;
