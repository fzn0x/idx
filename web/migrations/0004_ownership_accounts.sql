-- idx 0.8.2: a holding is a holder's register lines summed (`accounts` counts
-- them), the register as of 30 Oct 2026 adds CONTROLLER_OR_AFFILIATE, and a
-- month reloads when the mirror regenerates its parts (`parts_key`, the part
-- SHA-256s in order) even though the IDX file itself is unchanged.
ALTER TABLE own_holdings ADD COLUMN accounts INTEGER;
ALTER TABLE own_holdings ADD COLUMN controller_or_affiliate TEXT;
ALTER TABLE own_releases ADD COLUMN lines INTEGER;
ALTER TABLE own_releases ADD COLUMN other_date_rows INTEGER;
ALTER TABLE own_releases ADD COLUMN controller_column INTEGER;
ALTER TABLE own_releases ADD COLUMN parts_key TEXT;
