-- Existing events remain semantic changes. Only explicit future grouping paths
-- may preserve an earlier evidence generation; original history bytes stay intact.
ALTER TABLE memory_history ADD COLUMN grouping_only boolean NOT NULL DEFAULT false;
