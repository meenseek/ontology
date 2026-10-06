-- A lease does not identify its owner after the same revision is reclaimed.
-- Preserve existing queue rows and histories. Old processing rows have no token:
-- they expire into a new claim (<3 attempts) or error (>=3 attempts).
-- Stop every old writer before upgrading; this column cannot fence old SQL.
ALTER TABLE memory_grouping ADD COLUMN claim_token uuid;
