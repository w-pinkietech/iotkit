-- 計画4 T2(D7決定3/9): R11範囲クエリの正準時間軸。バックフィルは導出規則と同一
-- (未来方向許容ズレ300_000ms超のdevice_timeはreceived_atへ降格。過去方向に窓はない)。
ALTER TABLE readings ADD COLUMN event_time INTEGER NOT NULL DEFAULT 0;
ALTER TABLE readings ADD COLUMN event_time_source TEXT NOT NULL DEFAULT 'received_at';
UPDATE readings SET
    event_time = CASE
        WHEN device_time IS NOT NULL
             AND time_source IN ('device_ntp','device_rtc','edge_adjusted')
             AND device_time <= received_at + 300000
        THEN device_time ELSE received_at END,
    event_time_source = CASE
        WHEN device_time IS NOT NULL AND device_time <= received_at + 300000
             AND time_source IN ('device_ntp','device_rtc') THEN 'device'
        WHEN device_time IS NOT NULL AND device_time <= received_at + 300000
             AND time_source = 'edge_adjusted' THEN 'edge_adjusted'
        ELSE 'received_at' END;
CREATE INDEX idx_readings_series_event_time ON readings(series_id, event_time);
