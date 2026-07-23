-- H3: bound the sightings table before R2 opens ingress to untrusted senders.
-- Index serves both the TTL delete (WHERE last_seen < cutoff) and the LRU cap
-- eviction (ORDER BY last_seen DESC).
CREATE INDEX idx_sightings_last_seen ON sightings(last_seen);
